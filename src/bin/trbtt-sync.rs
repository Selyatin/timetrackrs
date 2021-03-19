use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_channel as chan;
use futures_util::{future, pin_mut, select, FutureExt, StreamExt};

use track_pc_usage_rs::sync::{MsgKind, PeerMsg};
use tungstenite::Message;
use uuid::Uuid;

use tokio::spawn;
use tokio::time::timeout;
use tokio_tungstenite::connect_async;

use datachannel::{
    Config, DataChannel, DataChannelInit, DescriptionType, IceCandidate, PeerConnection,
    Reliability, RtcDataChannel, RtcPeerConnection, SessionDescription,
};

// Server part

#[derive(Clone)]
struct DataPipe {
    output: chan::Sender<String>,
    ready: Option<chan::Sender<()>>,
}

impl DataPipe {
    fn new(output: chan::Sender<String>, ready: Option<chan::Sender<()>>) -> Self {
        DataPipe { output, ready }
    }
}

impl DataChannel for DataPipe {
    fn on_open(&mut self) {
        if let Some(ready) = &mut self.ready {
            ready.try_send(()).ok();
        }
    }

    fn on_message(&mut self, msg: &[u8]) {
        let msg = String::from_utf8_lossy(msg).to_string();
        self.output.try_send(msg).ok();
    }
}

struct WsConn {
    peer_id: Uuid,
    dest_id: Uuid,
    signaling: chan::Sender<Message>,
    dc: Option<Box<RtcDataChannel<DataPipe>>>,
}

impl WsConn {
    fn new(peer_id: Uuid, dest_id: Uuid, signaling: chan::Sender<Message>) -> Self {
        WsConn {
            peer_id,
            dest_id,
            signaling,
            dc: None,
        }
    }
}

impl PeerConnection for WsConn {
    type DC = DataPipe;

    fn on_description(&mut self, sess_desc: SessionDescription) {
        let peer_msg = PeerMsg {
            dest_id: self.dest_id,
            kind: MsgKind::Description(sess_desc),
        };

        self.signaling
            .try_send(Message::binary(serde_json::to_vec(&peer_msg).unwrap()))
            .ok();
    }

    fn on_candidate(&mut self, cand: IceCandidate) {
        let peer_msg = PeerMsg {
            dest_id: self.dest_id,
            kind: MsgKind::Candidate(cand),
        };

        self.signaling
            .try_send(Message::binary(serde_json::to_vec(&peer_msg).unwrap()))
            .ok();
    }

    fn on_data_channel(&mut self, mut dc: Box<RtcDataChannel<DataPipe>>) {
        log::info!(
            "Received Datachannel with: label={}, protocol={:?}, reliability={:?}",
            dc.label(),
            dc.protocol(),
            dc.reliability()
        );

        dc.send(format!("Hello from {}", self.peer_id).as_bytes())
            .ok();
        self.dc.replace(dc);
    }
}

type ConnectionMap = Arc<Mutex<HashMap<Uuid, Box<RtcPeerConnection<WsConn, DataPipe>>>>>;
type ChannelMap = Arc<Mutex<HashMap<Uuid, Box<RtcDataChannel<DataPipe>>>>>;

async fn run_client(peer_id: Uuid, input: chan::Receiver<Uuid>, output: chan::Sender<String>) {
    let conns = ConnectionMap::new(Mutex::new(HashMap::new()));
    let chans = ChannelMap::new(Mutex::new(HashMap::new()));

    let ice_servers = vec!["stun:stun.l.google.com:19302".to_string()];
    let conf = Config::new(ice_servers);

    let url = format!("ws://116.203.43.199:48749/{:?}", peer_id);
    // let url = format!("ws://127.0.0.1:48749/{:?}", peer_id);
    let (ws_stream, _) = connect_async(url).await.expect("Failed to connect");

    let (outgoing, mut incoming) = ws_stream.split();
    let (tx_ws, rx_ws) = chan::unbounded();

    let send = async {
        let dest_id = match input.recv().await {
            Ok(dest_id) if dest_id != peer_id => dest_id,
            Err(_) | Ok(_) => return,
        };
        log::info!("Peer {:?} sends data", &peer_id);

        let pipe = DataPipe::new(output.clone(), None);
        let conn = WsConn::new(peer_id, dest_id, tx_ws.clone());
        let pc = RtcPeerConnection::new(&conf, conn, pipe).unwrap();
        conns.lock().unwrap().insert(dest_id, pc);

        let (tx_ready, mut rx_ready) = chan::bounded(1);
        let pipe = DataPipe::new(output.clone(), Some(tx_ready));
        let opts = DataChannelInit::default()
            .protocol("prototest")
            .reliability(Reliability::default().unordered());

        let mut dc = conns
            .lock()
            .unwrap()
            .get_mut(&dest_id)
            .unwrap()
            .create_data_channel_ex("sender", pipe, &opts)
            .unwrap();
        rx_ready.next().await;

        let data = format!("Hello from {:?}", peer_id);
        dc.send(data.as_bytes()).ok();
        chans.lock().unwrap().insert(dest_id, dc);
    };

    let reply = rx_ws.map(Ok).forward(outgoing);

    let receive = async {
        while let Some(Ok(msg)) = incoming.next().await {
            if !msg.is_binary() {
                continue;
            }

            let peer_msg = match serde_json::from_slice::<PeerMsg>(&msg.into_data()) {
                Ok(peer_msg) => peer_msg,
                Err(err) => {
                    log::error!("Invalid PeerMsg: {}", err);
                    continue;
                }
            };
            let dest_id = peer_msg.dest_id;

            let mut locked = conns.lock().unwrap();
            let pc = match locked.get_mut(&dest_id) {
                Some(pc) => pc,
                None => match &peer_msg.kind {
                    MsgKind::Description(SessionDescription { desc_type, .. })
                        if matches!(desc_type, DescriptionType::Offer) =>
                    {
                        log::info!("Client {:?} answering to {:?}", &peer_id, &dest_id);

                        let pipe = DataPipe::new(output.clone(), None);
                        let conn = WsConn::new(peer_id, dest_id, tx_ws.clone());
                        let pc = RtcPeerConnection::new(&conf, conn, pipe).unwrap();

                        locked.insert(dest_id, pc);
                        locked.get_mut(&dest_id).unwrap()
                    }
                    _ => {
                        log::warn!("Peer {} not found in client", &dest_id);
                        continue;
                    }
                },
            };

            match &peer_msg.kind {
                MsgKind::Description(sess_desc) => pc.set_remote_description(sess_desc).ok(),
                MsgKind::Candidate(cand) => pc.add_remote_candidate(cand).ok(),
            };
        }
    };

    let send = send.fuse();
    pin_mut!(receive, reply, send);
    loop {
        select! {
            _ = future::select(&mut receive, &mut reply) => break,
            _ = &mut send => continue,
        }
    }

    conns.lock().unwrap().clear();
    chans.lock().unwrap().clear();
}

async fn go() -> anyhow::Result<()> {
    track_pc_usage_rs::util::init_logging();

    let id1 = Uuid::new_v4();
    let id2 = Uuid::new_v4();

    let (tx_res, rx_res) = chan::unbounded();
    let (tx_id, rx_id) = chan::bounded(2);

    spawn(run_client(id1, rx_id.clone(), tx_res.clone()));
    spawn(run_client(id2, rx_id.clone(), tx_res.clone()));

    let mut expected = HashSet::new();
    expected.insert(format!("Hello from {:?}", id1));
    expected.insert(format!("Hello from {:?}", id2));

    tx_id.try_send(id1).unwrap();
    tx_id.try_send(id1).unwrap();

    let mut res = HashSet::new();
    let r1 = timeout(Duration::from_secs(5), rx_res.recv()).await;
    let r2 = timeout(Duration::from_secs(5), rx_res.recv()).await;
    res.insert(r1.unwrap().unwrap());
    res.insert(r2.unwrap().unwrap());

    assert_eq!(expected, res);
    Ok(())
}
fn main() -> anyhow::Result<()> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(go())?;
    Ok(())
}

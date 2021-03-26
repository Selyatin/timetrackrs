use std::io::prelude::*;
use timetrackrs::capture::*;

use timetrackrs::prelude::*;
use typescript_definitions::TypeScriptifyTrait;

#[cfg(debug_assertions)]
const FS: &[fn() -> std::borrow::Cow<'static, str>] = &[
    // DbEvent::type_script_ify,
    EventData::type_script_ify,
    linux::types::X11EventData::type_script_ify,
    linux::types::X11WindowData::type_script_ify,
    linux::types::X11WindowGeometry::type_script_ify,
    linux::types::ProcessData::type_script_ify,
    linux::types::NetworkInfo::type_script_ify,
    linux::types::WifiInterface::type_script_ify,
    util::OsInfo::type_script_ify,
    TagRuleGroup::type_script_ify,
    TagRuleGroupData::type_script_ify,
    TagRuleWithMeta::type_script_ify,
    TagRule::type_script_ify,
    TagRuleGroupV1::type_script_ify,
    TagValue::type_script_ify,
    TagValueRegex::type_script_ify,
    TagAddReason::type_script_ify,
    api_types::ApiTypesTS::type_script_ify,
    api_types::SingleExtractedEvent::type_script_ify,
    api_types::SingleExtractedEventWithRaw::type_script_ify,
    api_types::ApiResponse::<String>::type_script_ify,
    Tags::type_script_ify,
];

#[cfg(not(debug_assertions))]
const FS: &'static [fn() -> std::borrow::Cow<'static, str>] = &[];

// const all_types: Vec<
fn main() -> anyhow::Result<()> {
    util::init_logging();

    let mut ofile = std::fs::File::create("frontend/src/server.d.ts")?;
    writeln!(ofile, "type DateTime<T> = string;")?;
    writeln!(ofile, "type Local = unknown;")?;
    writeln!(ofile, "type Timestamptz = number;")?;
    writeln!(ofile, "type Utc = void;")?;
    writeln!(ofile, "type Regex = string;")?;
    writeln!(ofile, "type ExternalFetcher = string;")?;
    writeln!(ofile, "type InternalFetcher = string;")?;
    if cfg!(any(debug_assertions, feature = "export-typescript")) {
        if FS.is_empty() {
            println!("Not in debug mode??");
        }
        for f in FS {
            writeln!(ofile, "{}", f())?;
        }
        println!("output {} types", FS.len());
    } else {
        println!("NOT IN DEBUG MODE, will not work!")
    }
    Ok(())
}

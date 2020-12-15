use crate::db::schema::config::*;
use crate::db::schema::extracted::*;
use crate::db::schema::raw_events::*;
use crate::prelude::*;
use diesel::serialize::{self, Output, ToSql};
use diesel::sql_types::Text;
use diesel::sqlite::Sqlite;
use diesel::{
    deserialize::{self, FromSql},
    sql_types::{BigInt, Double},
};
use std::io::Write;

#[derive(Queryable, Serialize, TypeScriptify)]
pub struct DbEvent {
    pub insertion_sequence: i64,
    pub id: String,
    pub timestamp: Timestamptz,
    pub data_type: String,
    pub sampler: Sampler,
    pub sampler_sequence_id: String,
    pub data: String,
}

impl DbEvent {
    pub fn deserialize_data(&self) -> anyhow::Result<EventData> {
        deserialize_captured((&self.data_type, &self.data))
            .with_context(|| format!("deserialization of event {}", self.id))
    }
}

#[derive(
    AsExpression, FromSqlRow, PartialEq, PartialOrd, Debug, Clone, Eq, Hash, Serialize, Deserialize,
)]
#[sql_type = "Text"]
pub struct Timestamptz(pub DateTime<Utc>);

impl FromSql<Text, Sqlite> for Timestamptz {
    fn from_sql(
        bytes: Option<&<Sqlite as diesel::backend::Backend>::RawValue>,
    ) -> deserialize::Result<Self> {
        let s = <String as FromSql<Text, Sqlite>>::from_sql(bytes)?;
        Ok(Timestamptz(util::iso_string_to_datetime(&s)?))
    }
}
impl ToSql<Text, Sqlite> for Timestamptz {
    fn to_sql<W: Write>(&self, out: &mut Output<W, Sqlite>) -> serialize::Result {
        let s = self.0.to_rfc3339();
        <String as ToSql<Text, Sqlite>>::to_sql(&s, out)
    }
}

#[derive(
    AsExpression, FromSqlRow, PartialEq, PartialOrd, Debug, Clone, Eq, Hash, Serialize, Deserialize,
)]
#[sql_type = "BigInt"]
pub struct TimestamptzI(pub DateTime<Utc>);
impl FromSql<BigInt, Sqlite> for TimestamptzI {
    fn from_sql(
        bytes: Option<&<Sqlite as diesel::backend::Backend>::RawValue>,
    ) -> deserialize::Result<Self> {
        let i = <i64 as FromSql<BigInt, Sqlite>>::from_sql(bytes)?;
        Ok(TimestamptzI(util::unix_epoch_millis_to_date(i)))
    }
}
impl ToSql<BigInt, Sqlite> for TimestamptzI {
    fn to_sql<W: Write>(&self, out: &mut Output<W, Sqlite>) -> serialize::Result {
        let s = self.0.timestamp_millis();
        <i64 as ToSql<BigInt, Sqlite>>::to_sql(&s, out)
    }
}
impl From<&Timestamptz> for TimestamptzI {
    fn from(t: &Timestamptz) -> Self {
        Self(t.0)
    }
}
impl From<&TimestamptzI> for Timestamptz {
    fn from(t: &TimestamptzI) -> Self {
        Self(t.0)
    }
}

#[derive(AsExpression, FromSqlRow, PartialEq, PartialOrd, Debug, Clone, Eq, Hash)]
#[sql_type = "Text"]
pub struct DateUtc(pub Date<Utc>);

impl FromSql<Text, Sqlite> for DateUtc {
    fn from_sql(
        bytes: Option<&<Sqlite as diesel::backend::Backend>::RawValue>,
    ) -> deserialize::Result<Self> {
        let s = <String as FromSql<Text, Sqlite>>::from_sql(bytes)?;
        Ok(DateUtc(util::iso_string_to_date(&s)?))
    }
}
impl ToSql<Text, Sqlite> for DateUtc {
    fn to_sql<W: Write>(&self, out: &mut Output<W, Sqlite>) -> serialize::Result {
        let s = format!("{}", self.0.format("%F"));
        <String as ToSql<Text, Sqlite>>::to_sql(&s, out)
    }
}

impl FromSql<Text, Sqlite> for Sampler {
    fn from_sql(
        bytes: Option<&<Sqlite as diesel::backend::Backend>::RawValue>,
    ) -> deserialize::Result<Self> {
        let s = <String as FromSql<Text, Sqlite>>::from_sql(bytes)?;
        Ok(serde_json::from_str(&s)?)
    }
}
impl ToSql<Text, Sqlite> for Sampler {
    fn to_sql<W: Write>(&self, out: &mut Output<W, Sqlite>) -> serialize::Result {
        let s = serde_json::to_string(&self)?;
        <String as ToSql<Text, Sqlite>>::to_sql(&s, out)
    }
}

#[derive(Insertable)]
#[table_name = "events"]
pub struct NewDbEvent {
    pub id: String,
    pub timestamp: Timestamptz,
    pub data_type: String,
    pub sampler: Sampler,
    pub sampler_sequence_id: String,
    pub data: String,
}

#[derive(
    Debug, Queryable, Insertable, Serialize, Deserialize, TypeScriptify, AsChangeset, Clone,
)]
#[table_name = "extracted_events"]
pub struct InExtractedTag {
    pub timestamp: TimestamptzI,
    pub duration: f64,
    pub event_id: i64,
    pub tag: i64,
    pub value: i64,
}
#[derive(Debug, Serialize, Deserialize, TypeScriptify, Clone, QueryableByName)]
pub struct OutExtractedTag {
    #[sql_type = "BigInt"]
    pub timestamp: TimestamptzI,
    #[sql_type = "Text"]
    pub event_id: String,
    #[sql_type = "Double"]
    pub duration: f64,
    #[sql_type = "Text"]
    pub tag: String,
    #[sql_type = "Text"]
    pub value: String,
}

#[derive(
    Debug,
    Queryable,
    Identifiable,
    Insertable,
    Serialize,
    Deserialize,
    TypeScriptify,
    AsChangeset,
    Clone,
)]
#[primary_key(global_id)]
#[table_name = "tag_rule_groups"]
pub struct TagRuleGroup {
    pub global_id: String,
    pub data: TagRuleGroupData,
}

#[derive(AsExpression, FromSqlRow, Debug, Serialize, Deserialize, TypeScriptify, Clone)]
#[sql_type = "Text"]
#[serde(tag = "version")]
pub enum TagRuleGroupData {
    V1 { data: TagRuleGroupV1 },
}

impl TagRuleGroupData {
    pub fn into_iter_active_rules(self) -> impl Iterator<Item = TagRule> {
        match self {
            TagRuleGroupData::V1 { data } => {
                data.rules
                    .into_iter()
                    .filter_map(|r| if r.enabled { Some(r.rule) } else { None })
            }
        }
    }
    pub fn into_iter_all_rules(self) -> impl Iterator<Item = TagRule> {
        match self {
            TagRuleGroupData::V1 { data } => data.rules.into_iter().map(|r| r.rule),
        }
    }
}

impl FromSql<Text, Sqlite> for TagRuleGroupData {
    fn from_sql(
        bytes: Option<&<Sqlite as diesel::backend::Backend>::RawValue>,
    ) -> deserialize::Result<Self> {
        let s = <String as FromSql<Text, Sqlite>>::from_sql(bytes)?;
        Ok(serde_json::from_str(&s)?)
    }
}
impl ToSql<Text, Sqlite> for TagRuleGroupData {
    fn to_sql<W: Write>(&self, out: &mut Output<W, Sqlite>) -> serialize::Result {
        let s = serde_json::to_string(&self)?;
        <String as ToSql<Text, Sqlite>>::to_sql(&s, out)
    }
}

// Copyright 2019 Materialize, Inc. All rights reserved.
//
// This file is part of Materialize. Materialize may not be used or
// distributed without the express permission of Materialize, Inc.

//! The types for the dataflow crate.
//!
//! These are extracted into their own crate so that crates that only depend
//! on the interface of the dataflow crate, and not its implementation, can
//! avoid the dependency, as the dataflow crate is very slow to compile.

use expr::{ColumnOrder, RelationExpr, ScalarExpr};
use repr::{Datum, RelationDesc, RelationType, Row};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;
use url::Url;
use uuid::Uuid;

/// System-wide update type.
pub type Diff = isize;

/// System-wide timestamp type.
pub type Timestamp = u64;

/// Specifies when a `Peek` should occur.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PeekWhen {
    /// The peek should occur at the latest possible timestamp that allows the
    /// peek to complete immediately.
    Immediately,
    /// The peek should occur at the latest possible timestamp that has been
    /// accepted by each input source.
    EarliestSource,
    /// The peek should occur at the specified timestamp.
    AtTimestamp(Timestamp),
}

/// The response from a `Peek`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PeekResponse {
    Rows(Vec<Row>),
    Canceled,
}

impl PeekResponse {
    pub fn unwrap_rows(self) -> Vec<Row> {
        match self {
            PeekResponse::Rows(rows) => rows,
            PeekResponse::Canceled => {
                panic!("PeekResponse::unwrap_rows called on PeekResponse::Canceled")
            }
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
/// A batch of updates to be fed to a local input
pub struct Update {
    pub row: Row,
    pub timestamp: u64,
    pub diff: isize,
}

pub fn compare_columns(order: &[ColumnOrder], left: &[Datum], right: &[Datum]) -> Ordering {
    for order in order {
        let (lval, rval) = (left[order.column], right[order.column]);
        let cmp = if order.desc {
            rval.cmp(&lval)
        } else {
            lval.cmp(&rval)
        };
        if cmp != Ordering::Equal {
            return cmp;
        }
    }
    Ordering::Equal
}

/// Instructions for finishing the result of a query.
///
/// The primary reason for the existence of this structure and attendant code
/// is that SQL's ORDER BY requires sorting rows (as already implied by the
/// keywords), whereas much of the rest of SQL is defined in terms of unordered
/// multisets. But as it turns out, the same idea can be used to optimize
/// trivial peeks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RowSetFinishing {
    /// Include only rows matching all predicates.
    pub filter: Vec<ScalarExpr>,
    /// Order rows by the given columns.
    pub order_by: Vec<ColumnOrder>,
    /// Include only as many rows (after offset).
    pub limit: Option<usize>,
    /// Omit as many rows.
    pub offset: usize,
    /// Include only given columns.
    pub project: Vec<usize>,
}

impl RowSetFinishing {
    pub fn is_trivial(&self) -> bool {
        (self.limit == None) && self.order_by.is_empty() && self.offset == 0
    }
}

#[derive(Debug, Clone)]
pub enum LocalInput {
    /// Send a batch of updates to the input
    Updates(Vec<Update>),
    /// All future updates will have timestamps >= this timestamp
    Watermark(u64),
}

/// A named stream of data.
#[serde(rename_all = "snake_case")]
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum Dataflow {
    Source(Source),
    Sink(Sink),
    View(View),
}

impl Dataflow {
    /// Reports the name of this dataflow.
    pub fn name(&self) -> &str {
        match self {
            Dataflow::Source(src) => &src.name,
            Dataflow::Sink(sink) => &sink.name,
            Dataflow::View(view) => &view.name,
        }
    }

    /// Reports the description of the datums produced by this dataflow.
    pub fn desc(&self) -> &RelationDesc {
        match self {
            Dataflow::Source(src) => &src.desc,
            Dataflow::Sink(_) => panic!(
                "programming error: Dataflow.typ called on Sink variant, \
                 but sinks don't have a type"
            ),
            Dataflow::View(view) => &view.desc,
        }
    }

    /// Reports the type of the datums produced by this dataflow.
    pub fn typ(&self) -> &RelationType {
        match self {
            Dataflow::Source(src) => src.desc.typ(),
            Dataflow::Sink(_) => panic!(
                "programming error: Dataflow.typ called on Sink variant, \
                 but sinks don't have a type"
            ),
            Dataflow::View(view) => view.desc.typ(),
        }
    }

    /// Collects the names of the dataflows that this dataflow depends upon.
    pub fn uses(&self) -> Vec<&str> {
        let mut out = Vec::new();
        match self {
            Dataflow::Source(_) => (),
            Dataflow::Sink(sink) => out.push(sink.from.0.as_str()),
            Dataflow::View(view) => view.relation_expr.unbound_uses(&mut out),
        }
        out
    }
}

/// A source materializes data. It typically represents an external source of
/// data, like a topic from Apache Kafka.
#[serde(rename_all = "snake_case")]
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub name: String,
    pub connector: SourceConnector,
    pub desc: RelationDesc,
}

#[serde(rename_all = "snake_case")]
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Sink {
    pub name: String,
    pub from: (String, RelationDesc),
    pub connector: SinkConnector,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SourceConnector {
    Kafka(KafkaSourceConnector),
    Local(LocalSourceConnector),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct KafkaSourceConnector {
    pub addr: std::net::SocketAddr,
    pub topic: String,
    pub raw_schema: String,
    #[serde(with = "url_serde")]
    pub schema_registry_url: Option<Url>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocalSourceConnector {
    pub uuid: Uuid,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum SinkConnector {
    Kafka(KafkaSinkConnector),
    Tail(TailSinkConnector),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct KafkaSinkConnector {
    pub addr: std::net::SocketAddr,
    pub topic: String,
    pub schema_id: i32,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TailSinkConnector {
    pub tx: comm::mpsc::Sender<Vec<Update>>,
    pub since: Timestamp,
}

/// A view transforms one dataflow into another.
#[serde(rename_all = "snake_case")]
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct View {
    pub name: String,
    /// The raw SQL used to create this view.
    pub raw_sql: String,
    pub relation_expr: RelationExpr,
    pub desc: RelationDesc,
    /// Indicates if sources can be advanced to a supplied frontier.
    /// Outputs will only be correct from this frontier onward.
    pub as_of: Option<Vec<Timestamp>>,
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;
    use std::error::Error;

    use super::*;
    use repr::{ColumnType, ScalarType};

    /// Verify that a basic relation_expr serializes and deserializes to JSON sensibly.
    #[test]
    fn test_roundtrip() -> Result<(), Box<dyn Error>> {
        let dataflow = Dataflow::View(View {
            name: "report".into(),
            raw_sql: "<none>".into(),
            relation_expr: RelationExpr::Project {
                outputs: vec![1, 2],
                input: Box::new(RelationExpr::Join {
                    inputs: vec![
                        RelationExpr::Get {
                            name: "orders".into(),
                            typ: RelationType::new(vec![ColumnType::new(ScalarType::Int64)]),
                        },
                        Box::new(RelationExpr::Union {
                            left: Box::new(RelationExpr::Get {
                                name: "customers2018".into(),
                                typ: RelationType::new(vec![ColumnType::new(ScalarType::Int64)]),
                            }),
                            right: Box::new(RelationExpr::Get {
                                name: "customers2019".into(),
                                typ: RelationType::new(vec![ColumnType::new(ScalarType::Int64)]),
                            }),
                        })
                        .distinct(),
                    ],
                    variables: vec![vec![(0, 0), (1, 0)]],
                }),
            },
            desc: RelationDesc::empty()
                .add_column("name", ScalarType::String)
                .add_column("quantity", ScalarType::String),
            as_of: None,
        });

        let decoded: Dataflow = serde_json::from_str(&serde_json::to_string_pretty(&dataflow)?)?;
        assert_eq!(decoded, dataflow);

        Ok(())
    }
}

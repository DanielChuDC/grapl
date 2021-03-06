use grapl_graph_descriptions::graph_description::*;
use grapl_graph_descriptions::process::ProcessState;
use serde::{Deserialize, Serialize};
use std::convert::TryFrom;

#[derive(Clone, Debug, Hash, Serialize, Deserialize)]
pub struct ProcessStop {
    process_id: u64,
    name: String,
    hostname: String,
    timestamp: u64,
}

impl TryFrom<ProcessStop> for Graph {
    type Error = String;

    fn try_from(process_stop: ProcessStop) -> Result<Self, Self::Error> {
        let terminated_process = ProcessBuilder::default()
            .process_name(process_stop.name)
            .hostname(process_stop.hostname)
            .state(ProcessState::Terminated)
            .process_id(process_stop.process_id)
            .terminated_timestamp(process_stop.timestamp)
            .build()?;

        let mut graph = Graph::new(process_stop.timestamp);
        graph.add_node(terminated_process);

        Ok(graph)
    }
}

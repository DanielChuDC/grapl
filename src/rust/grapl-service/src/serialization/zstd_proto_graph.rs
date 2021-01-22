use std::io::Cursor;

use grapl_graph_descriptions::graph_description::*;
use prost::EncodeError;
use sqs_executor::completion_event_serializer::CompletionEventSerializer;
use tracing::{debug,
              error};

/// Takes a subgraph generated by a Generator and serializes it so it can be returned by a
/// lambda as an execution result.
#[derive(Clone, Debug, Default)]
pub struct SubgraphSerializer {
    proto: Vec<u8>,
    compressed: Vec<u8>,
}

impl SubgraphSerializer {
    pub fn new(proto: Vec<u8>, compressed: Vec<u8>) -> Self {
        Self { proto, compressed }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum SubgraphSerializerError {
    #[error("IO")]
    Io(#[from] std::io::Error),
    #[error("EncodeError")]
    EncodeError(#[from] EncodeError),
}

impl CompletionEventSerializer for SubgraphSerializer {
    type CompletedEvent = Graph;
    type Output = Vec<u8>;
    type Error = SubgraphSerializerError;

    fn serialize_completed_events(
        &mut self,
        completed_events: &[Self::CompletedEvent],
    ) -> Result<Vec<Self::Output>, Self::Error> {
        let mut subgraph = Graph::new(0);

        // counts nodes and edges before subgraphs are merged
        let mut pre_nodes = 0;
        let mut pre_edges = 0;

        for sg in completed_events.iter() {
            pre_nodes += sg.nodes.len();
            pre_edges += sg.edges.len();
            subgraph.merge(sg);
        }

        if subgraph.is_empty() {
            debug!(
                concat!(
                    "Output subgraph is empty. Serializing to empty vector.",
                    "pre_nodes: {} pre_edges: {}"
                ),
                pre_nodes, pre_edges,
            );
            return Ok(vec![]);
        }

        // TODO: This should be a monitor
        debug!(
            "Serializing {} nodes {} edges. Down from {} nodes {} edges.",
            subgraph.nodes.len(),
            subgraph.edges.len(),
            pre_nodes,
            pre_edges,
        );

        let subgraphs = GeneratedSubgraphs {
            subgraphs: vec![subgraph],
        };

        self.proto.clear();
        self.compressed.clear();

        // encode generated subgraphs into protocol buffer
        prost::Message::encode(&subgraphs, &mut self.proto)?;

        let mut compressed = &mut self.compressed;
        let mut proto = std::io::Cursor::new(&self.proto);

        // compress encoded subgraph into `compressed` vector
        zstd::stream::copy_encode(&mut proto, &mut compressed, 4)?;

        Ok(vec![compressed.clone()])
    }
}

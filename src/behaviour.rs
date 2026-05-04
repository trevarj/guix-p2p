use libp2p::{
    identify,
    kad::{Behaviour as KadBehaviour, Config as KadConfig, Event as KadEvent, store::MemoryStore},
    mdns,
    request_response::{self, cbor},
    swarm::NetworkBehaviour,
};

use crate::swarm::codec::{BlockRequest, BlockResponse};

#[derive(NetworkBehaviour)]
#[behaviour(to_swarm = "GuixP2PEvent")]
pub struct GuixP2PBehaviour {
    pub kad: KadBehaviour<MemoryStore>,
    pub block_exchange: cbor::Behaviour<BlockRequest, BlockResponse>,
    pub mdns: mdns::tokio::Behaviour,
    pub identify: identify::Behaviour,
}

#[derive(Debug)]
pub enum GuixP2PEvent {
    Kad(KadEvent),
    BlockExchange(request_response::Event<BlockRequest, BlockResponse>),
    Mdns(mdns::Event),
    Identify(Box<identify::Event>),
}

impl From<identify::Event> for GuixP2PEvent {
    fn from(e: identify::Event) -> Self {
        Self::Identify(Box::new(e))
    }
}

impl From<KadEvent> for GuixP2PEvent {
    fn from(e: KadEvent) -> Self {
        Self::Kad(e)
    }
}

impl From<request_response::Event<BlockRequest, BlockResponse>> for GuixP2PEvent {
    fn from(e: request_response::Event<BlockRequest, BlockResponse>) -> Self {
        Self::BlockExchange(e)
    }
}

impl From<mdns::Event> for GuixP2PEvent {
    fn from(e: mdns::Event) -> Self {
        Self::Mdns(e)
    }
}

pub fn create_swarm_behaviour(keypair: &libp2p::identity::Keypair) -> GuixP2PBehaviour {
    let local_peer_id = libp2p::PeerId::from(keypair.public());

    let kad_config = KadConfig::default();
    let kad = KadBehaviour::with_config(local_peer_id, MemoryStore::new(local_peer_id), kad_config);

    let block_exchange = cbor::Behaviour::<BlockRequest, BlockResponse>::new(
        [(
            libp2p::StreamProtocol::new("/guix/substitute/0.1.0"),
            request_response::ProtocolSupport::Full,
        )],
        request_response::Config::default(),
    );

    let mdns = mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id)
        .expect("mdns setup should succeed");

    let identify = identify::Behaviour::new(
        identify::Config::new("/ipfs/0.1.0".into(), keypair.public())
            .with_agent_version(format!("guix-p2p-substitute/{}", env!("CARGO_PKG_VERSION"))),
    );

    GuixP2PBehaviour { kad, block_exchange, mdns, identify }
}

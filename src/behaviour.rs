use libp2p::{
    identify,
    kad::{
        Behaviour as KadBehaviour, Config as KadConfig, Event as KadEvent, Mode as KadMode,
        store::MemoryStore,
    },
    mdns,
    request_response::{self, cbor},
    swarm::{NetworkBehaviour, behaviour::toggle::Toggle},
};

use crate::swarm::codec::{BlockRequest, BlockResponse};

#[derive(NetworkBehaviour)]
#[behaviour(to_swarm = "GuixP2PEvent")]
pub struct GuixP2PBehaviour {
    pub kad: KadBehaviour<MemoryStore>,
    pub block_exchange: cbor::Behaviour<BlockRequest, BlockResponse>,
    pub mdns: Toggle<mdns::tokio::Behaviour>,
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

/// Create the default production swarm behaviour.
pub fn create_swarm_behaviour(keypair: &libp2p::identity::Keypair) -> GuixP2PBehaviour {
    create_swarm_behaviour_with_mdns(keypair, true)
}

/// Create a swarm behaviour with mDNS disabled.
///
/// Local E2E environments use explicit loopback bootstrap addresses, and some
/// containers deny multicast socket sends. Disabling mDNS there avoids noisy
/// permission errors without changing production defaults.
pub fn create_swarm_behaviour_without_mdns(
    keypair: &libp2p::identity::Keypair,
) -> GuixP2PBehaviour {
    create_swarm_behaviour_with_mdns(keypair, false)
}

fn create_swarm_behaviour_with_mdns(
    keypair: &libp2p::identity::Keypair,
    enable_mdns: bool,
) -> GuixP2PBehaviour {
    let local_peer_id = libp2p::PeerId::from(keypair.public());

    let kad_config = KadConfig::default();
    let mut kad =
        KadBehaviour::with_config(local_peer_id, MemoryStore::new(local_peer_id), kad_config);
    // guix-p2p nodes must answer provider lookups even in local E2E VMs that
    // do not advertise public addresses, so do not wait for Kad auto-promotion.
    kad.set_mode(Some(KadMode::Server));

    let block_exchange = cbor::Behaviour::<BlockRequest, BlockResponse>::new(
        [(
            libp2p::StreamProtocol::new("/guix/substitute/0.1.0"),
            request_response::ProtocolSupport::Full,
        )],
        request_response::Config::default(),
    );

    let mdns = if enable_mdns {
        match mdns::tokio::Behaviour::new(mdns::Config::default(), local_peer_id) {
            Ok(behaviour) => Some(behaviour),
            Err(e) => {
                // mDNS needs multicast socket permissions; containers often deny it.
                tracing::warn!("mDNS disabled: {}", e);
                None
            },
        }
    } else {
        None
    }
    .into();

    let identify = identify::Behaviour::new(
        identify::Config::new("/ipfs/0.1.0".into(), keypair.public())
            .with_agent_version(format!("guix-p2p/{}", crate::version::VERSION)),
    );

    GuixP2PBehaviour { kad, block_exchange, mdns, identify }
}

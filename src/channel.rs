use libp2p::PeerId;

use crate::swarm::codec::{BlockRequest, BlockResponse};

/// Commands sent from the daemon task to the swarm task.
#[derive(Debug)]
pub enum SwarmCommand {
    /// Initiate a DHT lookup for providers of a nar hash.
    GetProviders { hash: String },
    /// Send a block protocol request to a specific peer.
    SendBlockRequest { peer: PeerId, request: BlockRequest },
}

/// Notifications sent from the swarm task back to the daemon task.
#[derive(Debug)]
pub enum SwarmNotification {
    /// DHT lookup completed with these providers.
    ProvidersFound { hash: String, peers: Vec<PeerId> },
    /// A block protocol response arrived from a peer.
    BlockResponse { peer: PeerId, response: BlockResponse },
}

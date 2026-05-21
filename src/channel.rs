use libp2p::PeerId;

use crate::swarm::codec::{BlockRequest, BlockResponse};

/// Commands sent from the daemon task to the swarm task.
#[derive(Debug)]
pub enum SwarmCommand {
    /// Initiate a DHT lookup for providers of a nar hash.
    GetProviders { hash: String },
    /// Send a block protocol request to a specific peer.
    SendBlockRequest { peer: PeerId, request: BlockRequest },
    /// Announce this peer as a provider of a nar hash in the DHT.
    StartProviding { hash: String },
}

/// Notifications sent from the swarm task back to the daemon task.
/// Must be Clone for broadcast channel fan-out to socket connections.
#[derive(Debug, Clone)]
pub enum SwarmNotification {
    /// DHT lookup completed with these providers.
    ProvidersFound { hash: String, peers: Vec<PeerId> },
    /// A block protocol response arrived from a peer.
    BlockResponse { peer: PeerId, response: BlockResponse },
    /// A block protocol request failed before a response arrived.
    BlockRequestFailed { peer: PeerId },
}

/// Type alias for the broadcast sender used by the swarm task.
pub type NotifyTx = tokio::sync::broadcast::Sender<SwarmNotification>;

/// Type alias for the broadcast receiver used by daemon/relay connections.
pub type NotifyRx = tokio::sync::broadcast::Receiver<SwarmNotification>;

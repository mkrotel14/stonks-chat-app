//! Gateway connection: websocket worker, connection manager with the
//! authorization snapshot, event filtering and the authenticated REST calls
//! that back them. Hosts subscribe via [`ConnectionEvents`].

mod api;
mod gateway;
mod models;
mod state;
mod worker;

pub use api::{create_group, fetch_server_members, fetch_user, selected_session};
pub use gateway::GatewayEvent;
pub use models::{
    AppSnapshot, ClientCategory, ClientChannel, ClientFileReference, ClientMember, ClientMemberId,
    ClientRole, ClientServer, ClientStatus, ClientUnread, ClientUnreadId, ClientUser,
    ClientVoiceParticipant, ClientVoiceState, ConnectionStatus, ConnectionStatusReason,
    CreateGroupRequest, PermissionOverride, ServerMemberSnapshot,
};
pub use state::{parse_rfc3339, ConnectionManager};
pub use worker::{set_typing, start_connection, stop_connection, ConnectionEvents};

#[cfg(test)]
mod tests;

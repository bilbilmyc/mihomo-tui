mod connections;
#[cfg(test)]
use connections::MAX_CONNECTIONS;
pub use connections::{ConnectionSnapshot, ConnectionsView};

#[cfg(test)]
mod tests;

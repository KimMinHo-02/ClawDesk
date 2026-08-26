//! Windows system infrastructure adapters.

pub mod node_update;
pub mod system;

pub use node_update::NodeUpdateAdapter;
pub use system::WindowsSystemAdapter;

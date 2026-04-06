mod auth;
mod discovery;
mod model;
mod registry;

pub use auth::{AuthMode, AuthStore, OAuthCredential, StoredCredential};
pub use discovery::{merge_discovered_models, ModelDiscoveryClient};
pub use model::{
    ModelDescriptor, ModelDiscoveryConfig, ModelDiscoveryFormat, ProviderDescriptor,
    ProviderSource, ProviderSourceKind, RegisteredProvider,
};
pub use registry::ProviderRegistry;

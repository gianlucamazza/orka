#[allow(clippy::wildcard_imports)]
use crate::cmd::doctor::{
    DoctorCheck,
    checks::{
        architecture::*, config::*, connectivity::*, environment::*, providers::*, security::*,
    },
};

/// Build the ordered list of all registered doctor checks.
pub fn build_registry() -> Vec<Box<dyn DoctorCheck>> {
    vec![
        // Config — ordered (file exists before parsing before validation)
        Box::new(CfgFileExists),
        Box::new(CfgTomlParses),
        Box::new(CfgVersionCurrent),
        Box::new(CfgValidation),
        Box::new(CfgNoDeprecated),
        Box::new(CfgAgentDefs),
        Box::new(CfgGraphPresent),
        // Architecture
        Box::new(ArchLayeringViolations),
        Box::new(ArchCrateTestMinimum),
        Box::new(ArchOversizedModules),
        // Connectivity
        Box::new(ConRedisReachable),
        Box::new(ConRedisVersion),
        Box::new(ConQdrantReachable),
        Box::new(ConQdrantVersion),
        // Providers
        Box::new(PrvAtLeastOneProvider),
        Box::new(PrvApiKeysResolvable),
        Box::new(PrvProviderReachable),
        Box::new(PrvEmbeddingProvider),
        Box::new(PrvWebSearchKey),
        // Security
        Box::new(SecNoInlineKeys),
        Box::new(SecFilePermissions),
        Box::new(SecWorkspaceDirs),
        Box::new(SecSudoConfig),
        Box::new(SecNoNewPrivileges),
        // Environment
        Box::new(EnvRustToolchain),
        Box::new(EnvDockerAvailable),
        Box::new(EnvOsCapabilities),
        Box::new(EnvMcpBinaries),
        Box::new(EnvPluginDir),
        Box::new(EnvAdapterTokens),
    ]
}

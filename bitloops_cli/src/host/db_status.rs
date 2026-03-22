#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseConnectionStatus {
    Connected,
    CouldNotAuthenticate,
    CouldNotReachDb,
    NotConfigured,
    Error,
}

impl DatabaseConnectionStatus {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connected => "Connected",
            Self::CouldNotAuthenticate => "Could not authenticate",
            Self::CouldNotReachDb => "Could not reach DB",
            Self::NotConfigured => "Not configured",
            Self::Error => "Error",
        }
    }

    pub fn is_failure(self) -> bool {
        matches!(
            self,
            Self::CouldNotAuthenticate | Self::CouldNotReachDb | Self::Error
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatabaseStatusRow {
    pub db: &'static str,
    pub status: DatabaseConnectionStatus,
}

pub fn classify_connection_error(message: &str) -> DatabaseConnectionStatus {
    let lowered = message.to_ascii_lowercase();

    if contains_any(
        &lowered,
        &[
            "password authentication failed",
            "authentication failed",
            "could not authenticate",
            "invalid password",
            "invalid username",
            "unauthorized",
            "forbidden",
            "401",
            "403",
            "access denied",
            "no pg_hba.conf entry",
        ],
    ) {
        return DatabaseConnectionStatus::CouldNotAuthenticate;
    }

    if contains_any(
        &lowered,
        &[
            "could not connect",
            "connection refused",
            "failed to connect",
            "couldn't connect",
            "timed out",
            "timeout",
            "no route to host",
            "name or service not known",
            "temporary failure in name resolution",
            "network is unreachable",
            "operation not permitted",
            "connection to server at",
            "failed to lookup address information",
        ],
    ) {
        return DatabaseConnectionStatus::CouldNotReachDb;
    }

    DatabaseConnectionStatus::Error
}

fn contains_any(input: &str, markers: &[&str]) -> bool {
    markers.iter().any(|marker| input.contains(marker))
}

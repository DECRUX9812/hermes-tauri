#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    Probing { origin: String },
    Spawning { argv0: String },
    WaitingForLaunchLink,
    WaitingForHealth { origin: String },
    Ready { origin: String },
}

use std::time::Duration;

pub(crate) fn is_healthy(agent: &ureq::Agent, origin: &str, timeout: Duration) -> bool {
    match agent
        .get(&format!("{origin}/api/health"))
        .timeout(timeout)
        .call()
    {
        Ok(response) => (200..300).contains(&response.status()),
        Err(_) => false,
    }
}

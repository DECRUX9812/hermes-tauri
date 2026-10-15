use std::time::Duration;

/// True when the backend at `origin` is up.
///
/// `/api/health` is the webapp's own liveness route. The bundled `hermes_plus`
/// front door gates `/api/*` behind its session token, so an unauthenticated
/// probe gets 401/403 there; its open liveness route is `/healthz`
/// (`hermes_plus/server/auth.py` keeps `/healthz` public), which only answers
/// after its dashboard child and gateway are up, so a 2xx there means ready.
pub(crate) fn is_healthy(agent: &ureq::Agent, origin: &str, timeout: Duration) -> bool {
    match status(agent, &format!("{origin}/api/health"), timeout) {
        Some(status) if (200..300).contains(&status) => true,
        Some(401 | 403) => matches!(
            status(agent, &format!("{origin}/healthz"), timeout),
            Some(status) if (200..300).contains(&status)
        ),
        _ => false,
    }
}

fn status(agent: &ureq::Agent, url: &str, timeout: Duration) -> Option<u16> {
    match agent.get(url).timeout(timeout).call() {
        Ok(response) => Some(response.status()),
        Err(ureq::Error::Status(code, _)) => Some(code),
        Err(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::net::{TcpListener, TcpStream};
    use std::thread;

    /// A real socket answering fixed statuses per path. This is a fixture for
    /// the probe (the client side of the health check), not a stand-in for a
    /// Hermes backend: the statuses are the contract under test.
    fn stub_server(api_health: u16, healthz: u16) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub server");
        let origin = format!("http://{}", listener.local_addr().expect("addr"));
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { break };
                answer(stream, api_health, healthz);
            }
        });
        origin
    }

    fn answer(mut stream: TcpStream, api_health: u16, healthz: u16) {
        let Ok(clone) = stream.try_clone() else {
            return;
        };
        let mut reader = BufReader::new(clone);
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).is_err() {
            return;
        }
        // Drain the headers, so closing the socket cannot reset a client that is
        // still sending the request.
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) if line.trim().is_empty() => break,
                Ok(_) => {}
                Err(_) => return,
            }
        }
        let status = if request_line.starts_with("GET /healthz ") {
            healthz
        } else {
            api_health
        };
        let body = "{}";
        let response = format!(
            "HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(response.as_bytes());
    }

    fn agent() -> ureq::Agent {
        ureq::AgentBuilder::new().try_proxy_from_env(false).build()
    }

    #[test]
    fn an_open_health_route_is_ready() {
        let origin = stub_server(200, 404);
        assert!(is_healthy(&agent(), &origin, Duration::from_secs(2)));
    }

    #[test]
    fn a_gated_health_route_needs_the_open_liveness_route() {
        // hermes_plus: /api/health behind the token, /healthz open.
        let behind_a_front_door = stub_server(401, 200);
        assert!(is_healthy(
            &agent(),
            &behind_a_front_door,
            Duration::from_secs(2)
        ));
        // Still starting up: the front door answers, the liveness route does not.
        let starting_up = stub_server(401, 503);
        assert!(!is_healthy(&agent(), &starting_up, Duration::from_secs(2)));
    }

    #[test]
    fn a_missing_or_dead_origin_is_not_healthy() {
        // A closed port (bind then drop) and a server answering 404.
        let closed = {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
            let origin = format!("http://{}", listener.local_addr().expect("addr"));
            drop(listener);
            origin
        };
        assert!(!is_healthy(&agent(), &closed, Duration::from_secs(2)));
        let not_found = stub_server(404, 404);
        assert!(!is_healthy(&agent(), &not_found, Duration::from_secs(2)));
    }
}

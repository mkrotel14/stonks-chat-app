//! Duplicate application playback ports, never the mixed speaker monitor.
//! Links terminate at our private capture node and disappear with that node.
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use tokio::process::Command;

fn number(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| value.as_str()?.parse().ok())
}

fn excluded(value: &str) -> bool {
    let value = value.to_ascii_lowercase();
    value.contains("stonks") || value.contains("rdxlabs")
}

// Reject Stonks and helper ancestry even when graph labels are absent.
fn external_process(mut pid: u64) -> bool {
    for _ in 0..64 {
        if pid == 1 {
            return true;
        }
        if pid == 0 || pid == u64::from(std::process::id()) {
            return false;
        }
        let Ok(exe) = std::fs::read_link(format!("/proc/{pid}/exe")) else {
            return false;
        };
        let exe = exe.to_string_lossy();
        if excluded(&exe)
            || [
                "webkit",
                "pipewire",
                "pulseaudio",
                "pw-cat",
                "pw-play",
                "paplay",
                "bwrap",
            ]
            .iter()
            .any(|name| exe.to_ascii_lowercase().contains(name))
        {
            return false;
        }
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        let Some(parent) = stat
            .rsplit_once(')')
            .and_then(|(_, fields)| fields.split_whitespace().nth(1)?.parse::<u64>().ok())
        else {
            return false;
        };
        pid = parent;
    }
    false
}

// A Pulse client claim alone cannot establish origin. Corroborate it with the
// kernel's Unix socket table, and reject inherited/shared client endpoints.
fn pulse_connections(table: &str) -> HashSet<(u64, u64)> {
    let endpoints: Vec<_> = table
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            if fields.next()? != "u_str" || fields.next()? != "ESTAB" {
                return None;
            }
            let mut fields = fields.skip(2);
            let path = fields.next()?;
            let local = fields.next()?.parse::<u64>().ok()?;
            fields.next()?;
            let peer = fields.next()?.parse::<u64>().ok()?;
            let owners: HashSet<u64> = fields
                .collect::<String>()
                .split("pid=")
                .skip(1)
                .map(|s| s.split_once(',')?.0.parse().ok())
                .collect::<Option<_>>()?;
            if local == 0 || peer == 0 || owners.len() != 1 {
                return None;
            }
            Some((path, local, peer, owners.into_iter().next()?))
        })
        .collect();
    let by_inode: HashMap<_, _> = endpoints.iter().map(|e| (e.1, e)).collect();
    endpoints
        .iter()
        .filter_map(|(path, local, peer, broker)| {
            if !path.ends_with("/pulse/native") {
                return None;
            }
            let (_, _, reverse, app) = by_inode.get(peer)?;
            (reverse == local && app != broker).then_some((*broker, *app))
        })
        .collect()
}

fn verified_pulse_origin(
    connections: &HashSet<(u64, u64)>,
    broker: u64,
    app: u64,
    binary: &str,
) -> bool {
    if !connections.contains(&(broker, app)) {
        return false;
    }
    let exe = |pid| std::fs::read_link(format!("/proc/{pid}/exe")).ok();
    // On some distributions pipewire-pulse is a symlink to pipewire.
    let Ok(pulse) = std::fs::canonicalize("/usr/bin/pipewire-pulse") else {
        return false;
    };
    exe(broker).as_ref() == Some(&pulse)
        && exe(app).is_some_and(|path| path.file_name().is_some_and(|name| name == binary))
}

fn external(
    node: &Value,
    client: &Value,
    process: impl FnOnce(u64) -> bool,
    pulse: impl Fn(u64, u64, &str) -> bool,
) -> bool {
    let keys = [
        "application.id",
        "application.name",
        "application.process.binary",
        "node.name",
        "media.name",
    ];
    if [node, client].iter().any(|props| {
        keys.iter()
            .any(|key| props[*key].as_str().is_some_and(excluded))
    }) {
        return false;
    }
    let Some(peer) = number(&client["pipewire.sec.pid"]).filter(|pid| *pid > 1) else {
        return false;
    };
    let pid = if client["client.api"] == "pipewire-pulse" {
        if client["pulse.server.type"] != "unix" {
            return false;
        }
        let (Some(pid), Some(binary)) = (
            number(&client["application.process.id"]),
            client["application.process.binary"].as_str(),
        ) else {
            return false;
        };
        if !pulse(peer, pid, binary) {
            return false;
        }
        pid
    } else {
        peer
    };
    pid > 1
        && pid != u64::from(std::process::id())
        && [node, client].iter().all(|props| {
            props
                .get("application.process.id")
                .is_none_or(|claimed| number(claimed) == Some(pid))
        })
        && process(pid)
}

#[derive(Default, Debug)]
struct Plan {
    ready: bool,
    desired: HashSet<(u64, u64)>,
    existing: HashSet<(u64, u64)>,
}

fn plan(
    graph: &[Value],
    capture_name: &str,
    recorder_pid: u32,
    output_device: &str,
    process: impl Fn(u64) -> bool,
    pulse: impl Fn(u64, u64, &str) -> bool,
) -> Plan {
    let clients: HashMap<_, _> = graph
        .iter()
        .filter(|v| v["type"] == "PipeWire:Interface:Client")
        .filter_map(|v| Some((number(&v["id"])?, &v["info"]["props"])))
        .collect();
    let nodes: HashMap<_, _> = graph
        .iter()
        .filter(|v| v["type"] == "PipeWire:Interface:Node")
        .filter_map(|v| Some((number(&v["id"])?, &v["info"]["props"])))
        .collect();
    let capture = nodes
        .iter()
        .find(|(_, p)| {
            p["node.name"] == capture_name
                && p["media.class"] == "Stream/Input/Audio"
                && number(&p["client.id"])
                    .and_then(|id| clients.get(&id))
                    .and_then(|client| number(&client["pipewire.sec.pid"]))
                    == Some(u64::from(recorder_pid))
        })
        .map(|(id, _)| *id);
    let output_name = if output_device.is_empty() || output_device == "default" {
        graph
            .iter()
            .filter(|v| v["props"]["metadata.name"] == "default")
            .flat_map(|v| v["metadata"].as_array().into_iter().flatten())
            .find(|v| v["key"] == "default.audio.sink")
            .and_then(|v| v["value"]["name"].as_str())
    } else {
        Some(
            output_device
                .strip_prefix("pipewire:")
                .unwrap_or(output_device),
        )
    };
    let sink = nodes
        .iter()
        .find(|(_, p)| {
            p["media.class"] == "Audio/Sink"
                && p["node.name"].as_str() == output_name
                && output_name.is_some()
        })
        .map(|(id, _)| *id);
    let inputs: Vec<_> = graph
        .iter()
        .filter(|v| v["type"] == "PipeWire:Interface:Port")
        .filter(|v| {
            number(&v["info"]["props"]["node.id"]) == capture
                && capture.is_some()
                && v["info"]["props"]["port.direction"] == "in"
        })
        .filter_map(|v| {
            Some((
                number(&v["id"])?,
                v["info"]["props"]["audio.channel"].as_str()?,
            ))
        })
        .collect();
    let mut result = Plan {
        ready: inputs.len() == 2
            && inputs.iter().any(|(_, c)| *c == "FL")
            && inputs.iter().any(|(_, c)| *c == "FR")
            && sink.is_some(),
        ..Default::default()
    };
    let links: Vec<_> = graph
        .iter()
        .filter(|v| v["type"] == "PipeWire:Interface:Link")
        .map(|v| &v["info"])
        .collect();
    for link in &links {
        if number(&link["input-node-id"]) == capture && capture.is_some() {
            if let (Some(out), Some(input)) = (
                number(&link["output-port-id"]),
                number(&link["input-port-id"]),
            ) {
                result.existing.insert((out, input));
            }
        }
    }
    if !result.ready {
        return result;
    }
    // Only streams actually routed to the configured speaker/headphone sink.
    let playing: HashSet<_> = links
        .iter()
        .filter(|v| number(&v["input-node-id"]) == sink && sink.is_some())
        .filter_map(|v| Some((number(&v["output-node-id"])?, number(&v["output-port-id"])?)))
        .collect();
    for port in graph
        .iter()
        .filter(|v| v["type"] == "PipeWire:Interface:Port")
    {
        let p = &port["info"]["props"];
        let Some(id) = number(&port["id"]) else {
            continue;
        };
        let Some(node) = number(&p["node.id"])
            .filter(|node| playing.contains(&(*node, id)))
            .and_then(|id| nodes.get(&id))
        else {
            continue;
        };
        if node["media.class"] != "Stream/Output/Audio"
            || !number(&node["client.id"])
                .and_then(|id| clients.get(&id))
                .is_some_and(|client| external(node, client, &process, &pulse))
            || p["port.direction"] != "out"
            || p["port.monitor"] == true
            || p["port.monitor"] == "true"
        {
            continue;
        }
        for (input, channel) in &inputs {
            if p["audio.channel"] == *channel || p["audio.channel"] == "MONO" {
                result.desired.insert((id, *input));
            }
        }
    }
    result
}

async fn graph() -> Result<Vec<Value>, String> {
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        Command::new("pw-dump").kill_on_drop(true).output(),
    )
    .await
    .map_err(|_| "PipeWire graph inspection timed out.".to_owned())?
    .map_err(|e| format!("Could not inspect PipeWire playback: {e}"))?;
    if !output.status.success() {
        return Err("PipeWire playback graph is unavailable.".into());
    }
    serde_json::from_slice(&output.stdout).map_err(|e| format!("Invalid PipeWire graph: {e}"))
}

async fn apply(plan: Plan) -> Result<(), String> {
    for (out, input) in plan.desired.difference(&plan.existing) {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            Command::new("pw-link")
                .args([out.to_string(), input.to_string()])
                .kill_on_drop(true)
                .output(),
        )
        .await
        .map_err(|_| "PipeWire audio routing timed out.".to_owned())?
        .map_err(|e| format!("Could not route shared audio: {e}"))?;
        if !result.status.success() {
            return Err("A shared-audio stream changed while routing.".into());
        }
    }
    Ok(())
}

async fn inspect(capture: &str, pid: u32, output: &str) -> Result<Plan, String> {
    let graph = graph().await?;
    let connections = if graph.iter().any(|v| {
        v["type"] == "PipeWire:Interface:Client"
            && v["info"]["props"]["client.api"] == "pipewire-pulse"
    }) {
        tokio::time::timeout(
            std::time::Duration::from_millis(150),
            Command::new("ss")
                .args(["-xnpH"])
                .kill_on_drop(true)
                .output(),
        )
        .await
        .ok()
        .and_then(Result::ok)
        .filter(|output| output.status.success())
        .map(|output| pulse_connections(&String::from_utf8_lossy(&output.stdout)))
        .unwrap_or_default()
    } else {
        HashSet::new()
    };
    // /proc inspection is synchronous. Keep it off the async executor.
    let (capture, output) = (capture.to_owned(), output.to_owned());
    tokio::task::spawn_blocking(move || {
        plan(
            &graph,
            &capture,
            pid,
            &output,
            external_process,
            |broker, app, binary| verified_pulse_origin(&connections, broker, app, binary),
        )
    })
    .await
    .map_err(|_| "Could not verify PipeWire application origins.".to_owned())
}

pub(super) async fn start(capture: &str, pid: u32, output: &str) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(4);
    tokio::time::timeout_at(deadline, async {
        loop {
            let routes = inspect(capture, pid, output).await?;
            if !routes.existing.is_empty() {
                return Err("Private recorder was unexpectedly linked on startup.".into());
            }
            if routes.ready {
                let expected = routes.desired.clone();
                apply(routes).await?;
                return verify(capture, pid, output, &expected).await;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
    })
    .await
    .map_err(|_| "Private screen audio route did not become ready.".to_owned())?
}

async fn verify(
    capture: &str,
    pid: u32,
    output: &str,
    expected: &HashSet<(u64, u64)>,
) -> Result<(), String> {
    let routes = inspect(capture, pid, output).await?;
    if !routes.ready || !routes.existing.is_subset(&routes.desired) || &routes.existing != expected
    {
        return Err("Private screen audio route could not be verified.".into());
    }
    Ok(())
}

pub(super) async fn refresh(capture: &str, pid: u32, output: &str) -> Result<(), String> {
    let routes = inspect(capture, pid, output).await?;
    // Never continue sending buffered PCM after an input failed eligibility.
    if !routes.ready || !routes.existing.is_subset(&routes.desired) {
        return Err("Private screen audio route changed unexpectedly.".into());
    }
    if routes.existing == routes.desired {
        return Ok(());
    }
    let expected = routes.desired.clone();
    apply(routes).await?;
    verify(capture, pid, output, &expected).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn object(kind: &str, id: u64, props: Value) -> Value {
        json!({"type": format!("PipeWire:Interface:{kind}"), "id": id, "info": {"props": props}})
    }

    fn link(node: u64, port: u64, dest: u64, input: u64) -> Value {
        json!({"type": "PipeWire:Interface:Link", "info": {
            "output-node-id": node, "output-port-id": port,
            "input-node-id": dest, "input-port-id": input
        }})
    }

    #[test]
    fn routes_only_verified_application_ports_on_the_chosen_sink() {
        let mut graph = vec![
            object("Client", 1, json!({"pipewire.sec.pid": 41})),
            object("Client", 2, json!({"pipewire.sec.pid": 42})),
            object(
                "Node",
                10,
                json!({"client.id": 1, "node.name": "private", "media.class": "Stream/Input/Audio"}),
            ),
            object(
                "Node",
                20,
                json!({"node.name": "headphones", "media.class": "Audio/Sink"}),
            ),
            object(
                "Node",
                30,
                json!({"node.name": "other", "media.class": "Audio/Sink"}),
            ),
            object(
                "Node",
                40,
                json!({"client.id": 2, "media.class": "Stream/Output/Audio"}),
            ),
            object(
                "Node",
                50,
                json!({"client.id": 2, "media.class": "Stream/Output/Audio", "application.id": "com.rdxlabs.stonks"}),
            ),
            object("Node", 60, json!({"media.class": "Stream/Output/Audio"})),
        ];
        for (id, node, direction, channel) in [
            (11, 10, "in", "FL"),
            (12, 10, "in", "FR"),
            (31, 30, "out", "FL"),
            (43, 40, "out", "MONO"),
            (44, 40, "out", "FL"),
            (51, 50, "out", "FL"),
            (61, 60, "out", "FR"),
        ] {
            graph.push(object(
                "Port",
                id,
                json!({"node.id": node, "port.direction": direction, "audio.channel": channel}),
            ));
        }
        graph.extend([
            link(30, 31, 20, 21),
            link(40, 43, 20, 21),
            link(40, 44, 30, 32),
            link(50, 51, 20, 21),
            link(60, 61, 20, 22),
        ]);
        let routes = plan(
            &graph,
            "private",
            41,
            "pipewire:headphones",
            |pid| pid == 42,
            |_, _, _| false,
        );
        assert!(routes.ready);
        assert_eq!(routes.desired, HashSet::from([(43, 11), (43, 12)]));
        assert!(
            !plan(
                &graph,
                "private",
                42,
                "headphones",
                |_| true,
                |_, _, _| false
            )
            .ready
        );
        assert!(!plan(&graph, "private", 41, "missing", |_| true, |_, _, _| false).ready);
        graph.push(link(30, 31, 10, 11));
        let unexpected = plan(
            &graph,
            "private",
            41,
            "headphones",
            |_| true,
            |_, _, _| false,
        );
        assert!(!unexpected.existing.is_subset(&unexpected.desired));
    }

    #[test]
    fn pulse_browser_needs_matching_local_socket_and_external_ancestry() {
        let table = "u_str ESTAB 0 0 /run/user/1000/pulse/native 100 * 101 users:((\"pipewire-pulse\",pid=80,fd=9))
\
                     u_str ESTAB 0 0 * 101 * 100 users:((\"browser\",pid=42,fd=10))";
        assert_eq!(pulse_connections(table), HashSet::from([(80, 42)]));
        assert!(
            pulse_connections(&table.replace("fd=10))", "fd=10), (\"helper\",pid=43,fd=12))"))
                .is_empty()
        );
        assert!(pulse_connections(&table.replace("101 * 100", "101 * 102")).is_empty());
        let mut broker = json!({"pipewire.sec.pid": 80, "client.api": "pipewire-pulse",
            "pulse.server.type": "unix", "application.process.id": 42,
            "application.process.binary": "browser"});
        let node = json!({"application.process.id": "42"});
        let connections = pulse_connections(table);
        let corroborate =
            |peer, pid, binary: &str| connections.contains(&(peer, pid)) && binary == "browser";
        assert!(external(&node, &broker, |pid| pid == 42, corroborate));
        assert!(!external(&node, &broker, |_| false, corroborate));
        assert!(!external(&node, &broker, |_| true, |_, _, _| false));
        assert!(!external(
            &json!({"application.process.id": 43}),
            &broker,
            |_| true,
            corroborate
        ));
        assert!(!external(
            &json!({"application.id": "com.rdxlabs.stonks"}),
            &broker,
            |_| true,
            corroborate
        ));
        broker["pulse.server.type"] = json!("tcp");
        assert!(!external(&node, &broker, |_| true, corroborate));
    }

    #[test]
    fn fail_closed_on_unknown_or_conflicting_origin() {
        let empty = json!({});
        let peer = json!({"pipewire.sec.pid": 42});
        assert!(!external(&empty, &empty, |_| true, |_, _, _| false));
        assert!(!external(&empty, &peer, |_| false, |_, _, _| false));
        assert!(!external(
            &json!({"application.process.id": 99}),
            &peer,
            |_| true,
            |_, _, _| false
        ));
        assert!(!external(
            &empty,
            &json!({"pipewire.sec.pid": std::process::id()}),
            |_| true,
            |_, _, _| false
        ));
        for name in ["Stonks", "Stonks Chat", "stonks-app", "com.rdxlabs.stonks"] {
            assert!(!external(
                &json!({"application.process.binary": name}),
                &peer,
                |_| true,
                |_, _, _| false
            ));
        }
    }
}

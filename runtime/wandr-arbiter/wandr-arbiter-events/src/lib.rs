//! wandr-arbiter-events — the generic in-process event broker (task 90).
//!
//! One topic → subscribers registry + a retained value per topic. This is the
//! broker behind the `wandr:events` WIT bus: the host's `producer.publish` and the
//! wandr-net daemon publish here (`evt-publish <topic> <payload>`); guests are
//! registered as subscribers by the host from their `package.toml [events]
//! subscribe` (`evt-subscribe <pid> <topic>`), and receive each event as an
//! `event <topic> <payload>` line via [`Effect::HostLine`] → the host's standalone
//! drain → the guest's `wandr:events/incoming-handler.handle`.
//!
//! Retained delivery (MQTT-style): on subscribe, the topic's last published value
//! is delivered immediately, so a guest that subscribes after the publisher started
//! is still coherent (e.g. it learns "online" without waiting for the next change).
//!
//! The payload is an opaque, already-base64'd string on the wire — the broker
//! stores + fans it verbatim and never decodes it (it's topic-schema'd, decoded by
//! the guest). Never touches binder/sockets — desktop-testable via the
//! `evt-publish` / `evt-subscribe` verbs with no device.

use std::collections::HashMap;

use wandr_arbiter_core::{ArbiterModule, Ctx, Event, Reply};

#[derive(Default)]
pub struct EventsModule {
    /// topic → subscriber pids (the guests' control-socket pids).
    subscribers: HashMap<String, Vec<i32>>,
    /// topic → last-published payload (base64), delivered to new subscribers.
    retained: HashMap<String, String>,
}

impl EventsModule {
    pub fn new() -> Self {
        Self::default()
    }

    /// `evt-subscribe <pid> <topic>` — register a guest for a topic. Immediately
    /// delivers the topic's retained value (if any) so the guest starts coherent.
    fn cmd_subscribe(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let mut it = args.split_whitespace();
        let (Some(pid), Some(topic)) = (
            it.next().and_then(|t| t.parse::<i32>().ok()),
            it.next(),
        ) else {
            return Reply::err("evt-subscribe-usage <pid> <topic>");
        };
        let subs = self.subscribers.entry(topic.to_string()).or_default();
        if !subs.contains(&pid) {
            subs.push(pid);
        }
        if let Some(payload) = self.retained.get(topic) {
            ctx.deliver_to_host(pid, format!("event {topic} {payload}\n"));
        }
        Reply::ok(format!("evt-subscribe {pid} {topic}"))
    }

    /// `evt-unsubscribe <pid> <topic>` — drop a guest from a topic.
    fn cmd_unsubscribe(&mut self, args: &str) -> Reply {
        let mut it = args.split_whitespace();
        let (Some(pid), Some(topic)) = (
            it.next().and_then(|t| t.parse::<i32>().ok()),
            it.next(),
        ) else {
            return Reply::err("evt-unsubscribe-usage <pid> <topic>");
        };
        if let Some(subs) = self.subscribers.get_mut(topic) {
            subs.retain(|&p| p != pid);
        }
        Reply::ok(format!("evt-unsubscribe {pid} {topic}"))
    }

    /// `evt-publish <topic> <payload>` — store the retained value + fan to every
    /// subscriber of `topic`. `payload` is opaque (base64); fanned verbatim.
    /// An empty payload is allowed (a topic with no data, e.g. a bare signal).
    fn cmd_publish(&mut self, args: &str, ctx: &mut Ctx) -> Reply {
        let mut it = args.trim().splitn(2, ' ');
        let Some(topic) = it.next().filter(|t| !t.is_empty()) else {
            return Reply::err("evt-publish-usage <topic> <payload-base64>");
        };
        let payload = it.next().unwrap_or("").to_string();
        self.retained.insert(topic.to_string(), payload.clone());
        let n = self.subscribers.get(topic).map(|s| s.len()).unwrap_or(0);
        if let Some(subs) = self.subscribers.get(topic).cloned() {
            for pid in subs {
                ctx.deliver_to_host(pid, format!("event {topic} {payload}\n"));
            }
        }
        log::info!("events: publish topic={topic} → {n} subscriber(s)");
        Reply::ok(format!("evt-publish {topic} subscribers={n}"))
    }
}

impl ArbiterModule for EventsModule {
    fn verbs(&self) -> &[&'static str] {
        &["evt-subscribe", "evt-unsubscribe", "evt-publish"]
    }

    fn on_command(&mut self, verb: &str, args: &str, ctx: &mut Ctx) -> Reply {
        match verb {
            "evt-subscribe" => self.cmd_subscribe(args, ctx),
            "evt-unsubscribe" => self.cmd_unsubscribe(args),
            "evt-publish" => self.cmd_publish(args, ctx),
            other => Reply::err(format!("events-unknown-verb {other}")),
        }
    }

    fn on_event(&mut self, ev: &Event, _ctx: &mut Ctx) {
        // A subscribed guest's process exited — drop it from every topic so we stop
        // pushing to a dead pid (mirrors the net module's subscriber cleanup).
        if let Event::SurfaceRemoved { pid } = ev {
            for subs in self.subscribers.values_mut() {
                subs.retain(|&p| p != *pid);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::EventsModule;
    use wandr_arbiter_core::{Effect, Event, Registry, Store};

    fn reg() -> (Registry, Store) {
        let mut r = Registry::new();
        r.register(Box::new(EventsModule::new()));
        (r, Store::new())
    }

    /// Collect the `event …` lines a batch of effects would deliver, as `pid:line`.
    fn host_lines(effects: &[Effect]) -> Vec<String> {
        effects
            .iter()
            .filter_map(|e| match e {
                Effect::HostLine { pid, line } => Some(format!("{pid}:{}", line.trim())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn publish_fans_to_subscribers_only() {
        let (mut r, mut store) = reg();
        r.dispatch_command("evt-subscribe", "10 net.status", &mut store).unwrap();
        r.dispatch_command("evt-subscribe", "20 other.topic", &mut store).unwrap();
        let (_reply, effects) = r.dispatch_command("evt-publish", "net.status b25saW5l", &mut store).unwrap();
        // Only pid 10 (subscribed to net.status) gets the push.
        assert_eq!(host_lines(&effects), vec!["10:event net.status b25saW5l"]);
    }

    #[test]
    fn subscribe_delivers_retained_value_immediately() {
        let (mut r, mut store) = reg();
        // Publish before anyone subscribes — value is retained.
        r.dispatch_command("evt-publish", "net.status QUJD", &mut store).unwrap();
        // A late subscriber gets the retained value right away.
        let (_reply, effects) = r.dispatch_command("evt-subscribe", "30 net.status", &mut store).unwrap();
        assert_eq!(host_lines(&effects), vec!["30:event net.status QUJD"]);
    }

    #[test]
    fn dead_pid_dropped_on_surface_removed() {
        let (mut r, mut store) = reg();
        r.dispatch_command("evt-subscribe", "40 net.status", &mut store).unwrap();
        r.dispatch_event(Event::SurfaceRemoved { pid: 40 }, &mut store);
        let (_reply, effects) = r.dispatch_command("evt-publish", "net.status eA==", &mut store).unwrap();
        assert!(host_lines(&effects).is_empty());
    }
}

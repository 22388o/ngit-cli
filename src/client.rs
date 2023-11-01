// have you considered

// TO USE ASYNC

// in traits (required for mocking unit tests)
// https://rust-lang.github.io/async-book/07_workarounds/05_async_in_traits.html
// https://github.com/dtolnay/async-trait
// see https://blog.rust-lang.org/inside-rust/2022/11/17/async-fn-in-trait-nightly.html
// I think we can use the async-trait crate and switch to the native feature
// which is currently in nightly. alternatively we can use nightly as it looks
// certain that the implementation is going to make it to stable but we don't
// want to inadvertlty use other features of nightly that might be removed.
use anyhow::{Context, Result};
use async_trait::async_trait;
use futures::future::join_all;
#[cfg(test)]
use mockall::*;
use nostr::Event;

pub struct Client {
    client: nostr_sdk::Client,
    fallback_relays: Vec<String>,
}

#[cfg_attr(test, automock)]
#[async_trait]
pub trait Connect {
    fn default() -> Self;
    fn new(opts: Params) -> Self;
    async fn set_keys(&mut self, keys: &nostr::Keys);
    async fn disconnect(&self) -> Result<()>;
    fn get_fallback_relays(&self) -> &Vec<String>;
    async fn send_event_to(&self, url: &str, event: nostr::event::Event) -> Result<nostr::EventId>;
    async fn get_events(
        &self,
        relays: Vec<String>,
        filters: Vec<nostr::Filter>,
    ) -> Result<Vec<nostr::Event>>;
}

#[async_trait]
impl Connect for Client {
    fn default() -> Self {
        Client {
            client: nostr_sdk::Client::new(&nostr::Keys::generate()),
            fallback_relays: vec![
                "ws://localhost:8051".to_string(),
                "ws://localhost:8052".to_string(),
            ],
        }
    }
    fn new(opts: Params) -> Self {
        Client {
            client: nostr_sdk::Client::new(&opts.keys.unwrap_or(nostr::Keys::generate())),
            fallback_relays: opts.fallback_relays,
        }
    }

    async fn set_keys(&mut self, keys: &nostr::Keys) {
        self.client.set_keys(keys).await;
    }

    async fn disconnect(&self) -> Result<()> {
        self.client.disconnect().await?;
        Ok(())
    }

    fn get_fallback_relays(&self) -> &Vec<String> {
        &self.fallback_relays
    }

    async fn send_event_to(&self, url: &str, event: Event) -> Result<nostr::EventId> {
        self.client.add_relay(url, None).await?;
        self.client.connect_relay(url).await?;
        Ok(self.client.send_event_to(url, event).await?)
    }

    async fn get_events(
        &self,
        relays: Vec<String>,
        filters: Vec<nostr::Filter>,
    ) -> Result<Vec<nostr::Event>> {
        // add relays
        for relay in &relays {
            self.client
                .add_relay(relay.as_str(), None)
                .await
                .context("cannot add relay")?;
        }

        let relays_map = self.client.relays().await;

        let relay_results = join_all(
            relays
                .clone()
                .iter()
                .map(|r| {
                    (
                        relays_map.get(&nostr::Url::parse(r).unwrap()).unwrap(),
                        filters.clone(),
                    )
                })
                .map(|(relay, filters)| get_events_of(relay, filters)),
        )
        .await;

        Ok(get_dedup_events(relay_results))
    }
}

async fn get_events_of(
    relay: &nostr_sdk::Relay,
    filters: Vec<nostr::Filter>,
) -> Result<Vec<Event>> {
    if !relay.is_connected().await {
        relay.connect(true).await;
    }
    relay
        .get_events_of(
            filters,
            // 20 is nostr_sdk default
            std::time::Duration::from_secs(20),
            nostr_sdk::FilterOptions::ExitOnEOSE,
        )
        .await
        .context("failed to get events from relay")
}

#[derive(Default)]
pub struct Params {
    pub keys: Option<nostr::Keys>,
    pub fallback_relays: Vec<String>,
}

impl Params {
    pub fn with_keys(mut self, keys: nostr::Keys) -> Self {
        self.keys = Some(keys);
        self
    }
    pub fn with_fallback_relays(mut self, fallback_relays: Vec<String>) -> Self {
        self.fallback_relays = fallback_relays;
        self
    }
}

fn get_dedup_events(relay_results: Vec<Result<Vec<nostr::Event>>>) -> Vec<Event> {
    let mut dedup_events: Vec<Event> = vec![];
    for events in relay_results.into_iter().flatten() {
        for event in events {
            if !dedup_events.iter().any(|e| event.id.eq(&e.id)) {
                dedup_events.push(event);
            }
        }
    }
    dedup_events
}

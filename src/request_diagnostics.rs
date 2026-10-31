//! Request timings, projected into each account's own channel namespace.
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::time::Instant;
use uuid::Uuid;

pub fn elapsed_ms(start: Instant) -> i32 {
    start.elapsed().as_millis().min(i32::MAX as u128) as i32
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Pending,
    Success,
    HttpError,
    Timeout,
    /// A stream produced no output in time and was failed over.
    FirstTokenTimeout,
    ConnectionError,
    StreamError,
    ClientDisconnected,
    Unavailable,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Attempt {
    pub channel_id: Uuid,
    pub channel_name: Option<String>,
    /// Offset from gateway entry. Other times below are relative to this attempt.
    pub started_ms: i32,
    pub duration_ms: i32,
    pub headers_ms: Option<i32>,
    pub first_token_ms: Option<i32>,
    pub status: Option<u16>,
    pub outcome: Outcome,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Diagnostics {
    /// Source-key owners do not own the caller's queue, so receive None.
    pub queue_ms: Option<i32>,
    pub attempts: Vec<Attempt>,
    pub completion: Option<Outcome>,
    /// bili reference tags removed from the history; absent when none were.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acp_stripped: Option<u32>,
}

pub struct Timings {
    started: Instant,
    owner: Uuid,
    queue_ms: i32,
    // Each relay hop's (source account, channel) for an attempt, outermost
    // first. They stay in memory, never in another account's JSON.
    attempts: Vec<(Attempt, Vec<(Uuid, Uuid)>)>,
    completion: Option<Outcome>,
    acp_stripped: u32,
}

impl Timings {
    pub fn new(started: Instant, owner: Uuid, queue_ms: i32) -> Self {
        Self {
            started,
            owner,
            queue_ms,
            attempts: Vec::new(),
            completion: None,
            acp_stripped: 0,
        }
    }

    pub fn acp_stripped(&mut self, count: u32) {
        self.acp_stripped = count;
    }

    pub fn begin(&mut self, channel: Uuid, hops: Vec<(Uuid, Uuid)>) {
        self.attempts.push((
            Attempt {
                channel_id: channel,
                channel_name: None,
                started_ms: elapsed_ms(self.started),
                duration_ms: 0,
                headers_ms: None,
                first_token_ms: None,
                status: None,
                outcome: Outcome::Pending,
            },
            hops,
        ));
    }

    pub fn headers(&mut self, status: u16) {
        if let Some((attempt, _)) = self.attempts.last_mut() {
            attempt.headers_ms = Some(elapsed_ms(self.started) - attempt.started_ms);
            attempt.status = Some(status);
        }
    }

    pub fn unavailable(&mut self, channel: Uuid, since: Instant, status: u16) {
        self.begin(channel, Vec::new());
        if let Some((attempt, _)) = self.attempts.last_mut() {
            attempt.started_ms = since
                .duration_since(self.started)
                .as_millis()
                .min(i32::MAX as u128) as i32;
            attempt.status = Some(status);
        }
        self.end_attempt(Outcome::Unavailable);
    }

    pub fn first_output(&mut self, total_ms: i32) {
        if let Some((attempt, _)) = self.attempts.last_mut() {
            attempt.first_token_ms = Some(total_ms.saturating_sub(attempt.started_ms));
        }
    }

    pub fn end_attempt(&mut self, outcome: Outcome) {
        if let Some((attempt, _)) = self.attempts.last_mut() {
            attempt.duration_ms = elapsed_ms(self.started) - attempt.started_ms;
            attempt.outcome = outcome;
        }
    }

    pub fn finish(&mut self, outcome: Outcome) {
        if self.completion.is_none() {
            if self
                .attempts
                .last()
                .is_some_and(|(a, _)| a.outcome == Outcome::Pending)
            {
                self.end_attempt(outcome);
            }
            self.completion = Some(outcome);
        }
    }

    pub fn finish_routing(&mut self) {
        self.finish(
            self.attempts
                .last()
                .map(|(a, _)| a.outcome)
                .unwrap_or(Outcome::Unavailable),
        );
    }

    fn for_account(&self, account: Uuid) -> Diagnostics {
        Diagnostics {
            queue_ms: (account == self.owner).then_some(self.queue_ms),
            attempts: self
                .attempts
                .iter()
                .filter_map(|(attempt, hops)| {
                    let mut attempt = attempt.clone();
                    if account != self.owner {
                        // A path visits an account at most once.
                        attempt.channel_id = hops
                            .iter()
                            .find(|(source_account, _)| *source_account == account)?
                            .1;
                    }
                    Some(attempt)
                })
                .collect(),
            completion: self.completion.or_else(|| {
                self.attempts
                    .last()
                    .map(|(attempt, _)| attempt.outcome)
                    .filter(|outcome| *outcome != Outcome::Pending)
            }),
            // The request body belongs to the caller, not a relay's source.
            acp_stripped: (account == self.owner && self.acp_stripped > 0)
                .then_some(self.acp_stripped),
        }
    }

    pub async fn save(&self, state: &AppState, events: &[(Uuid, Uuid)]) {
        for &(event, account) in events {
            let _ = sqlx::query(
                "UPDATE usage_events SET request_diagnostics=$3 WHERE id=$1 AND account_id=$2",
            )
            .bind(event)
            .bind(account)
            .bind(sqlx::types::Json(self.for_account(account)))
            .execute(&state.db)
            .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_diagnostics_never_expose_another_accounts_channels() {
        let owner = Uuid::new_v4();
        let source = Uuid::new_v4();
        let direct = Uuid::new_v4();
        let relay = Uuid::new_v4();
        let upstream = Uuid::new_v4();
        let mut timing = Timings::new(Instant::now(), owner, 42);
        timing.acp_stripped(0);
        timing.begin(direct, Vec::new());
        timing.end_attempt(Outcome::Timeout);
        timing.begin(relay, vec![(source, upstream)]);
        timing.headers(200);
        timing.finish(Outcome::Success);
        let own = timing.for_account(owner);
        assert_eq!(own.queue_ms, Some(42));
        assert_eq!(own.attempts.len(), 2);
        assert_eq!(own.attempts[1].channel_id, relay);
        assert_eq!(own.acp_stripped, None);
        let other = timing.for_account(source);
        assert_eq!(other.queue_ms, None);
        assert_eq!(other.attempts.len(), 1);
        assert_eq!(other.attempts[0].channel_id, upstream);
        let json = serde_json::to_string(&other).unwrap();
        assert!(!json.contains(&relay.to_string()));
        assert!(!json.contains(&direct.to_string()));
        assert!(timing.for_account(Uuid::new_v4()).attempts.is_empty());
        assert!(
            !serde_json::to_string(&own)
                .unwrap()
                .contains("acp_stripped")
        );
        timing.acp_stripped(2);
        assert_eq!(timing.for_account(owner).acp_stripped, Some(2));
        assert_eq!(timing.for_account(source).acp_stripped, None);
    }

    #[test]
    fn each_relay_hop_sees_only_its_own_channel() {
        let (owner, middle, last) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let (relay, inner_relay, upstream) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let mut timing = Timings::new(Instant::now(), owner, 0);
        timing.begin(relay, vec![(middle, inner_relay), (last, upstream)]);
        timing.finish(Outcome::Success);
        assert_eq!(timing.for_account(owner).attempts[0].channel_id, relay);
        assert_eq!(
            timing.for_account(middle).attempts[0].channel_id,
            inner_relay
        );
        assert_eq!(timing.for_account(last).attempts[0].channel_id, upstream);
        let json = serde_json::to_string(&timing.for_account(last)).unwrap();
        assert!(!json.contains(&relay.to_string()));
        assert!(!json.contains(&inner_relay.to_string()));
    }
}

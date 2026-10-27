//! The downloads' side of "Download only through a VPN": the gate before a download talks to
//! the swarm, and the kill switch. See [`crate::vpn`] for the rule.

use std::sync::atomic::Ordering;

use super::phase::{TorrentStart, TorrentState};
use super::{TorrentEvent, TorrentsSubsystem};
use crate::vpn::{pick, vpn_chosen, vpn_only};

impl TorrentsSubsystem {
    /// Downloads go only through the VPN. Never where the system can't bind them to one.
    pub(super) fn vpn_only(&self) -> bool {
        vpn_only(self.inner.records.config(), self.inner.ports.network.get().as_ref())
    }

    /// Some download waits for the VPN.
    pub(super) fn any_waiting_vpn(&self) -> bool {
        self.state().jobs.values().any(|j| j.waiting_vpn)
    }

    /// Some plugin waits for the VPN, with the setting on.
    pub fn plugin_waits_for_vpn(&self) -> bool {
        self.vpn_only() && self.inner.vpn_waited.load(Ordering::SeqCst)
    }

    /// The interface downloads may use now: `Ok(None)` for any, while the setting is off.
    /// `Err` while they wait for the VPN. The VPNs it reads go to the core, for the view.
    async fn wanted_route(&self) -> Result<Option<String>, ()> {
        if !self.vpn_only() {
            return Ok(None);
        }
        let network = self.inner.ports.network.get();
        let finds = network.finds_vpns();
        let vpns = tokio::task::spawn_blocking(move || network.vpns()).await.unwrap_or_default();
        let route = pick(&vpns, &vpn_chosen(self.inner.records.config()), finds);
        self.emit(TorrentEvent::VpnsRead(vpns));
        route.map(Some).ok_or(())
    }

    /// Marks download `id` waiting for the VPN: paused, unless it is finished.
    fn wait_for_vpn(&self, id: &str) {
        self.update(id, |job| {
            if job.state != TorrentState::Complete {
                job.state = TorrentState::Paused;
            }
            job.waiting_vpn = true;
            job.seeding = false;
            job.verifying = false;
            job.download_rate = 0;
            job.upload_rate = 0;
            job.peers = 0;
        });
    }

    /// Pauses every running download for the VPN, and closes the torrent port's connections.
    async fn stop_for_vpn(&self) {
        let running: Vec<String> = self.state().jobs.values().filter(|j| j.running()).map(|j| j.id.clone()).collect();
        if running.is_empty() {
            return;
        }
        for id in &running {
            self.wait_for_vpn(id);
        }
        if let Err(e) = self.port().stop_all().await {
            tracing::warn!(error = %e, "could not stop the downloads for the VPN");
        }
        tracing::info!("downloads paused: the VPN they use isn't connected");
    }

    /// Binds the torrent port to `route`. What runs on the old route stops and waits.
    async fn reroute(&self, route: Option<String>) {
        self.stop_for_vpn().await;
        match self.port().route(route.clone()).await {
            Ok(()) => self.state().vpn_bound = route,
            Err(e) => tracing::warn!(error = %e, "could not bind the downloads to the VPN"),
        }
    }

    /// Before a download talks to the swarm: binds the torrent port to the VPN. Without the
    /// VPN it marks download `id` waiting and returns `false`.
    pub(super) async fn vpn_gate(&self, id: &str) -> bool {
        let bound = self.state().vpn_bound.clone();
        if !self.vpn_only() && bound.is_none() {
            return true;
        }
        match self.wanted_route().await {
            Ok(route) if route == bound => true,
            Ok(route) => {
                self.reroute(route).await;
                // What the new route stopped starts at the next check.
                self.watch();
                true
            }
            Err(()) => {
                self.wait_for_vpn(id);
                self.watch();
                false
            }
        }
    }

    /// The kill switch, once a VPN poll while a download runs or waits, and after each change
    /// to the setting or the VPNs.
    pub(super) async fn check_vpn(&self) {
        if std::mem::replace(&mut self.state().vpn_checking, true) {
            return;
        }
        let mut back = false;
        match self.wanted_route().await {
            Err(()) => self.stop_for_vpn().await,
            Ok(route) => {
                if route != self.state().vpn_bound {
                    self.reroute(route).await;
                }
                self.resume_waiting().await;
                back = self.vpn_only() && self.inner.vpn_waited.swap(false, Ordering::SeqCst);
            }
        }
        self.state().vpn_checking = false;
        if back {
            self.emit(TorrentEvent::VpnBack);
        }
    }

    /// Starts again what the VPN stopped, and only that.
    async fn resume_waiting(&self) {
        let waiting: Vec<(String, TorrentState, usize)> = self
            .state()
            .jobs
            .values()
            .filter(|j| j.waiting_vpn)
            .map(|j| (j.id.clone(), j.state, j.file_count))
            .collect();
        for (id, state, files) in waiting {
            self.update(&id, |job| job.waiting_vpn = false);
            if files == 0 {
                self.read_again(&id).await;
            } else if state == TorrentState::Complete {
                if self.sharing() {
                    self.start(&id, TorrentStart::SeedOnly).await;
                }
            } else {
                self.start(&id, TorrentStart::Download { seed: self.sharing() }).await;
            }
        }
    }

    /// The setting or the VPNs changed: the kill switch looks at once. A plugin that waits
    /// for the VPN gets it this way too, for example when the player picks the VPN.
    pub async fn vpn_changed(&self) {
        if self.vpn_only() || self.any_waiting_vpn() || self.state().vpn_bound.is_some() {
            self.check_vpn().await;
            self.watch();
        }
    }
}

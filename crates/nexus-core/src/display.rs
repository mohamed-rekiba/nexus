//! The text the view models show: sizes, dates, durations and counts in words, as the player
//! reads them. The view builds every display string with these; themes show them as they are.

use nexus_ui_api::{AppView, GameCardVm, GameDetailVm, TorrentVm};

/// Writes the view's display text from the facts the view holds: each `*_text` field, and the
/// sizes, dates and times a theme shows in words. The view calls it once, when it is built.
pub(crate) fn write(view: &mut AppView) {
    let home = &mut view.home;
    let groups = home.groups.iter_mut().flat_map(|g| g.games.iter_mut());
    let cards = home.games.iter_mut().chain(groups).chain(&mut home.library.games).chain(&mut view.catalog.games.games);
    cards.for_each(card);
    if let Some(detail) = &mut view.detail {
        game(detail);
    }
    view.catalog.updated_text = date_text(&view.catalog.updated);
    for job in &mut view.downloads.jobs {
        job.progress_text = progress_text(job.received, job.total);
    }
    for t in &mut view.downloads.torrents {
        t.detail = torrent_detail(t);
        t.file_summary = file_summary(t);
    }
    let page = view.plugin_page.as_mut();
    let folders = view.settings.folders.iter_mut().chain(page.into_iter().flat_map(|p| p.folders.iter_mut()));
    for folder in folders {
        folder.requested_by_text = folder.requested_by.join(", ");
    }
    let page = view.plugin_page.as_mut().and_then(|p| p.emulator.as_mut());
    for emulator in view.emulators.emulators.iter_mut().chain(page) {
        emulator.controllers_text = emulator.controllers.join(", ");
    }
    if let Some(running) = &mut view.running {
        running.elapsed = duration_text(running.elapsed_secs);
    }
}

fn card(card: &mut GameCardVm) {
    (card.size, card.last_played, card.availability_text) =
        game_text(card.size_bytes, card.last_played_days, &card.availability);
}

fn game(d: &mut GameDetailVm) {
    (d.size, d.last_played, d.availability_text) = game_text(d.size_bytes, d.last_played_days, &d.availability);
    d.rating_text = rating_text(d.rating);
    d.release_text = date_text(&d.release);
    d.year = d.release.get(..4).unwrap_or_default().to_owned();
    d.played = played_text(d.played_secs);
    if let Some(compat) = &mut d.compat {
        compat.tested_text = date_text(&compat.tested);
    }
    for r in &mut d.releases {
        r.size = maybe_size_text(r.size_bytes);
        r.added_text = date_text(&r.added);
        r.counted_text = date_text(r.counted_at.get(..10).unwrap_or_default());
        r.availability_text = availability_text(&r.availability);
    }
}

/// The text a game's card and its page both show: its size, when it was last played, and
/// its availability, in that order.
fn game_text(size_bytes: Option<u64>, last_played_days: Option<u64>, availability: &str) -> (String, String, String) {
    (maybe_size_text(size_bytes), days_text(last_played_days), availability_text(availability))
}

/// `7.6 GB`, `820 MB`: decimal units, as the PS5 shows them.
pub(crate) fn size_text(bytes: u64) -> String {
    let gb = bytes as f64 / 1e9;
    if gb >= 1.0 { format!("{gb:.1} GB") } else { format!("{:.0} MB", bytes as f64 / 1e6) }
}

/// [`size_text`], or empty when the size isn't known.
fn maybe_size_text(bytes: Option<u64>) -> String {
    bytes.map(size_text).unwrap_or_default()
}

/// `4 hr 6 min`, `12 min`, or `Less than a minute`.
fn duration_text(secs: u64) -> String {
    let (hours, minutes) = (secs / 3600, secs % 3600 / 60);
    match (hours, minutes) {
        (0, 0) => "Less than a minute".into(),
        (0, m) => format!("{m} min"),
        (h, 0) => format!("{h} hr"),
        (h, m) => format!("{h} hr {m} min"),
    }
}

/// The time played: [`duration_text`], or empty when the game never ran.
fn played_text(secs: u64) -> String {
    if secs > 0 { duration_text(secs) } else { String::new() }
}

/// `2 hr 5 min left`, `12 min left`, or `under a minute left`.
fn time_left_text(secs: u64) -> String {
    if secs < 60 { "under a minute left".into() } else { format!("{} left", duration_text(secs)) }
}

/// `Today`, `Yesterday` or `3 days ago`; empty when the game never ran.
fn days_text(days: Option<u64>) -> String {
    match days {
        None => String::new(),
        Some(0) => "Today".into(),
        Some(1) => "Yesterday".into(),
        Some(n) => format!("{n} days ago"),
    }
}

/// `2024-03-12` as `Mar 12, 2024`, as the old launcher showed dates; empty stays empty.
fn date_text(date: &str) -> String {
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let day = date.get(8..10).and_then(|d| d.parse::<u32>().ok());
    let month = date.get(5..7).and_then(|m| m.parse::<usize>().ok()).filter(|m| (1..=12).contains(m));
    match (date.get(..4), month, day) {
        (Some(y), Some(m), Some(d)) => format!("{} {d}, {y}", MONTHS[m - 1]),
        _ => date.to_owned(),
    }
}

/// An availability in words: `good` is "Good", `unknown` is "Unknown"; empty stays empty.
fn availability_text(level: &str) -> String {
    match level {
        "good" => "Good",
        "low" => "Low",
        "none" => "None",
        "unknown" => "Unknown",
        _ => "",
    }
    .into()
}

/// `4.4 · 3.2K ratings`, or empty for none.
fn rating_text(rating: Option<nexus_api::Rating>) -> String {
    let Some(r) = rating else { return String::new() };
    let noun = if r.total == 1 { "rating" } else { "ratings" };
    format!("{:.1} · {} {noun}", r.score, short_count(r.total))
}

/// `950`, `3.2K`, `10.6K`, `2M`: one decimal, none when it is 0.
fn short_count(n: u64) -> String {
    let (value, unit) = match n {
        0..1_000 => return n.to_string(),
        // Up to where one decimal would round to 1000.0K.
        1_000..999_950 => (n as f64 / 1_000.0, "K"),
        _ => (n as f64 / 1_000_000.0, "M"),
    };
    let text = format!("{value:.1}");
    format!("{}{unit}", text.strip_suffix(".0").unwrap_or(&text))
}

/// `12.5 MB of 40.0 MB`, or `12.5 MB` when the size is unknown.
fn progress_text(received: u64, total: Option<u64>) -> String {
    let mb = |b: u64| format!("{:.1} MB", b as f64 / 1_000_000.0);
    match total {
        Some(t) => format!("{} of {}", mb(received), mb(t)),
        None => mb(received),
    }
}

/// `3.1 MB/s`.
fn rate_text(bytes: u64) -> String {
    format!("{:.1} MB/s", bytes as f64 / 1e6)
}

/// One line about a download: its sizes, speed and peers, by its state.
fn torrent_detail(t: &TorrentVm) -> String {
    let peers = if t.peers == 1 { "1 peer".to_owned() } else { format!("{} peers", t.peers) };
    match t.state.as_str() {
        "resolving" => "Reading the torrent's files from its peers; nothing downloads yet".into(),
        "ready" => "Look at its files below: nothing downloads until you press Download".into(),
        "checking" => "Checking the pieces already on disk".into(),
        "downloading" => {
            let left =
                if t.seconds_left > 0 { format!(" · {}", time_left_text(t.seconds_left)) } else { String::new() };
            format!("{} of {} · {} · {peers}{left}", size_text(t.done), size_text(t.total), rate_text(t.download_rate))
        }
        "paused" if t.waiting_vpn && t.file_count == 0 => {
            "It reads the torrent's files once your VPN is connected".into()
        }
        "paused" if t.waiting_vpn => {
            format!("{} of {} · it goes on once your VPN is connected", size_text(t.done), size_text(t.total))
        }
        "paused" | "cancelled" => format!("{} of {}", size_text(t.done), size_text(t.total)),
        "complete" if t.install == "installing" => {
            format!("Installing · {}% · it runs in the background", (t.install_progress * 100.0).round() as u32)
        }
        "complete" if t.unpack == "unpacking" => {
            format!("Unpacking its archives · {}%", (t.unpack_progress * 100.0).round() as u32)
        }
        "complete" if t.unpack == "cancelled" => "Unpacking stopped: Install unpacks the archives again".into(),
        "complete" if t.verifying => format!("{} · checking the files before sharing them", size_text(t.total)),
        "complete" if t.seeding => {
            format!("{} · sharing at {} · {peers}", size_text(t.total), rate_text(t.upload_rate))
        }
        "complete" => size_text(t.total),
        _ => String::new(),
    }
}

/// What the file list shows: `120 files · 1.5 GB · first 100 shown`.
fn file_summary(t: &TorrentVm) -> String {
    let files = if t.file_count == 1 { "1 file".to_owned() } else { format!("{} files", t.file_count) };
    let shown = if t.files.len() < t.file_count { format!(" · first {} shown", t.files.len()) } else { String::new() };
    format!("{files} · {}{shown}", size_text(t.total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_availability_reads_in_words() {
        let words: Vec<String> = ["good", "low", "none", "unknown", ""].into_iter().map(availability_text).collect();
        assert_eq!(words, ["Good", "Low", "None", "Unknown", ""]);
    }

    #[test]
    fn progress_text_reads_naturally() {
        assert_eq!(progress_text(12_500_000, Some(40_000_000)), "12.5 MB of 40.0 MB");
        assert_eq!(progress_text(0, None), "0.0 MB");
    }

    #[test]
    fn a_date_reads_as_month_day_and_year() {
        assert_eq!(date_text("2024-03-12"), "Mar 12, 2024");
        assert_eq!(date_text(""), "");
    }

    #[test]
    fn a_rating_says_how_many_players_gave_it() {
        assert_eq!(rating_text(Some(nexus_api::Rating { score: 4.42, total: 3_200 })), "4.4 · 3.2K ratings");
        assert_eq!(rating_text(Some(nexus_api::Rating { score: 5.0, total: 1 })), "5.0 · 1 rating");
        assert_eq!(rating_text(None), "");
    }

    #[test]
    fn a_download_s_line_says_its_install_s_percent_while_it_installs() {
        let t = TorrentVm {
            state: "complete".into(),
            seeding: true,
            install: "installing".into(),
            install_progress: 0.43,
            ..Default::default()
        };
        assert_eq!(torrent_detail(&t), "Installing · 43% · it runs in the background");
    }

    #[test]
    fn a_download_s_file_list_says_how_many_files_it_shows() {
        let files = vec!["a · 1 MB".to_owned(); 100];
        let t = TorrentVm { files, file_count: 120, total: 1_500_000_000, ..Default::default() };
        assert_eq!(file_summary(&t), "120 files · 1.5 GB · first 100 shown");
        let one = TorrentVm { files: vec!["a".into()], file_count: 1, total: 2_000_000, ..t };
        assert_eq!(file_summary(&one), "1 file · 2 MB");
    }

    #[test]
    fn a_download_s_line_says_its_time_left_only_while_bytes_arrive() {
        let t = TorrentVm {
            state: "downloading".into(),
            done: 1_000_000_000,
            total: 3_000_000_000,
            download_rate: 2_000_000,
            seconds_left: 7_500,
            peers: 3,
            ..Default::default()
        };
        assert_eq!(torrent_detail(&t), "1.0 GB of 3.0 GB · 2.0 MB/s · 3 peers · 2 hr 5 min left");
        assert!(torrent_detail(&TorrentVm { seconds_left: 30, ..t.clone() }).ends_with("under a minute left"));
        let stalled = TorrentVm { download_rate: 0, seconds_left: 0, ..t };
        assert_eq!(torrent_detail(&stalled), "1.0 GB of 3.0 GB · 0.0 MB/s · 3 peers");
    }
}

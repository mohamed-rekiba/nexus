use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id};
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::Registry;
use tracing_subscriber::layer::{Context, Layer, SubscriberExt};

/// Records every span opened on the current thread while its guard lives: the span name and
/// its fields as text.
///
/// Keep a test that uses it in its own test binary: `tracing` caches callsite interest
/// across threads, so spans can be missed while other tests run in parallel.
#[derive(Clone, Default)]
pub struct SpanRecorder {
    spans: Arc<Mutex<Vec<RecordedSpan>>>,
}

/// A span's name and its fields as text.
type RecordedSpan = (String, Vec<(String, String)>);

impl SpanRecorder {
    /// Starts recording on this thread. Use it with a current-thread tokio runtime.
    pub fn install(&self) -> DefaultGuard {
        tracing::subscriber::set_default(Registry::default().with(self.clone()))
    }

    /// Spans named `name`, as their fields.
    pub fn spans(&self, name: &str) -> Vec<Vec<(String, String)>> {
        self.spans.lock().unwrap().iter().filter(|(n, _)| n == name).map(|(_, f)| f.clone()).collect()
    }
}

struct Fields(Vec<(String, String)>);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.push((field.name().to_owned(), format!("{value:?}")));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.push((field.name().to_owned(), value.to_owned()));
    }
}

impl<S: tracing::Subscriber> Layer<S> for SpanRecorder {
    fn on_new_span(&self, attrs: &Attributes<'_>, _id: &Id, _ctx: Context<'_, S>) {
        let mut fields = Fields(Vec::new());
        attrs.record(&mut fields);
        self.spans.lock().unwrap().push((attrs.metadata().name().to_owned(), fields.0));
    }
}

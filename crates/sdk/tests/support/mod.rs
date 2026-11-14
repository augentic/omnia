//! The tracing recorder the suites read the `handler` span through.

use std::sync::{Arc, Mutex};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id};
use tracing::subscriber::DefaultGuard;
use tracing_subscriber::Registry;
use tracing_subscriber::layer::{Context as LayerContext, Layer, SubscriberExt as _};

type Span = (String, Vec<(String, String)>);

/// Every span opened under the recorder, by name, with the fields it recorded.
#[derive(Default)]
pub struct Spans(Arc<Mutex<Vec<Span>>>);

impl Spans {
    /// Install a recorder as the thread's default subscriber for the guard's life.
    pub fn install() -> (Self, DefaultGuard) {
        let layer = Self::default();
        let handle = Self(Arc::clone(&layer.0));
        let guard = tracing::subscriber::set_default(Registry::default().with(layer));
        (handle, guard)
    }

    /// The fields of the first `handler` span opened.
    pub fn handler(&self) -> Vec<(String, String)> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .find(|(name, _)| name == "handler")
            .map(|(_, fields)| fields.clone())
            .expect("a `handler` span is opened")
    }
}

impl<S: tracing::Subscriber> Layer<S> for Spans {
    fn on_new_span(&self, attrs: &Attributes<'_>, _id: &Id, _ctx: LayerContext<'_, S>) {
        struct Fields(Vec<(String, String)>);

        impl Visit for Fields {
            fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
                self.0.push((field.name().to_owned(), format!("{value:?}")));
            }

            fn record_str(&mut self, field: &Field, value: &str) {
                self.0.push((field.name().to_owned(), value.to_owned()));
            }
        }

        let mut fields = Fields(Vec::new());
        attrs.record(&mut fields);
        self.0.lock().unwrap().push((attrs.metadata().name().to_owned(), fields.0));
    }
}

/// `(name, value)` pairs as the recorder holds them.
pub fn fields<const N: usize>(pairs: [(&str, &str); N]) -> Vec<(String, String)> {
    pairs.into_iter().map(|(name, value)| (name.to_owned(), value.to_owned())).collect()
}

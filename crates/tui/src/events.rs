//! The sole terminal input owner; theme changes wake an otherwise idle terminal.
use async_channel::Receiver;
use crossterm::event::{Event, EventStream};
use futures_lite::{StreamExt, future::race};
use lenz::theme::ThemeUpdate;
use std::{io, time::Duration};
#[allow(clippy::large_enum_variant)] // Bounded stack value avoids allocating for every input.
pub enum Wake {
    Input(io::Result<Event>),
    Theme(ThemeUpdate),
    Work,
    Closed,
    ThemesClosed,
}
pub async fn wait(
    input: &mut EventStream,
    themes: &Receiver<ThemeUpdate>,
    work: Option<Duration>,
) -> Wake {
    let activity = race(
        async { input.next().await.map_or(Wake::Closed, Wake::Input) },
        async {
            if themes.is_closed() && themes.is_empty() {
                std::future::pending().await
            } else {
                themes.recv().await.map_or(Wake::ThemesClosed, Wake::Theme)
            }
        },
    );
    if let Some(delay) = work {
        race(activity, async {
            async_io::Timer::after(delay).await;
            Wake::Work
        })
        .await
    } else {
        activity.await
    }
}

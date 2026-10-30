//! External audio inputs as the player reads them: an input channel's sound is the frames of its
//! [`InputFeed`]. Live, a feed is the device ring the input callback fills (`dawsome-media`); in
//! a test or an offline render, frames given up front; with none, the channel is silent.
//!
//! The player reads every feed it holds once per frame it puts out, playing or stopped, heard or
//! muted, in channel order, so output frame k after a set of feeds arrives plays frame k of each
//! (no delay is added here) and a feed never falls behind while its channel is silent.

use crate::project::ChannelId;

/// The audio half of an input channel's source. The player calls [`read`](Self::read) on the
/// audio thread, so it must not allocate, free, lock, log or wait.
pub trait InputFeed: Send {
    /// How many channels the input has: 1 (a mono sound, through the channel pan law) or 2 (a
    /// stereo sound, through the balance law).
    fn channels(&self) -> usize;

    /// Writes the next `out.len()` frames of the input into `out`, a mono input's on both sides.
    /// A frame the feed does not have (an underrun, or past its end) is silence; a frame is
    /// never given twice.
    fn read(&mut self, out: &mut [[f32; 2]]);
}

/// The feeds the player reads, at most one per input channel, kept in channel order (the order
/// the player reads and mixes them in). Built on the UI thread and handed over whole with
/// [`crate::realtime::RealtimeLink::set_input_feeds`]; the set it replaces comes back to be
/// dropped there.
#[derive(Default)]
pub struct InputFeeds {
    feeds: Vec<(ChannelId, Box<dyn InputFeed>)>,
}

impl InputFeeds {
    pub fn new() -> Self {
        Self::default()
    }

    /// Makes `feed` channel `channel`'s, in place of any it had.
    pub fn insert(&mut self, channel: ChannelId, feed: Box<dyn InputFeed>) {
        match self.feeds.binary_search_by_key(&channel, |(id, _)| *id) {
            Ok(index) => self.feeds[index].1 = feed,
            Err(index) => self.feeds.insert(index, (channel, feed)),
        }
    }

    pub fn len(&self) -> usize {
        self.feeds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.feeds.is_empty()
    }

    /// Each channel with its feed, in channel order.
    pub(crate) fn iter_mut(
        &mut self,
    ) -> impl Iterator<Item = (ChannelId, &mut (dyn InputFeed + 'static))> {
        self.feeds.iter_mut().map(|(id, feed)| (*id, feed.as_mut()))
    }

    /// The channels that have a feed, in order.
    pub(crate) fn channels(&self) -> impl Iterator<Item = ChannelId> + '_ {
        self.feeds.iter().map(|(id, _)| *id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Level(f32);

    impl InputFeed for Level {
        fn channels(&self) -> usize {
            1
        }

        fn read(&mut self, out: &mut [[f32; 2]]) {
            out.fill([self.0; 2]);
        }
    }

    // A set keeps one feed per channel, in channel order, whatever order they come in.
    #[test]
    fn a_set_holds_one_feed_per_channel_in_channel_order() {
        let mut feeds = InputFeeds::new();
        feeds.insert(ChannelId(3), Box::new(Level(0.3)));
        feeds.insert(ChannelId(1), Box::new(Level(0.1)));
        feeds.insert(ChannelId(3), Box::new(Level(0.5)));
        assert_eq!(feeds.len(), 2);
        assert_eq!(
            feeds.channels().collect::<Vec<_>>(),
            [ChannelId(1), ChannelId(3)]
        );
        let mut out = [[0.0; 2]; 1];
        let levels: Vec<f32> = feeds
            .iter_mut()
            .map(|(_, feed)| {
                feed.read(&mut out);
                out[0][0]
            })
            .collect();
        assert_eq!(levels, [0.1, 0.5]);
    }
}

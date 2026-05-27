//! Telegram notification mute helpers.

use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use grammers_client::types::{Dialog, Message};
use grammers_tl_types as tl;

/// Returns whether a dialog's current peer notification settings mute messages.
pub(crate) fn dialog_notifications_muted(dialog: &Dialog) -> bool {
    dialog_is_muted_at(dialog, now_unix_seconds())
}

/// Tracks peer notification mute state observed from dialogs and raw updates.
#[derive(Debug, Default)]
pub(crate) struct NotificationMuteCache {
    muted_chat_ids: BTreeSet<i64>,
}

impl NotificationMuteCache {
    /// Records a dialog's current notification mute state and returns whether it is muted.
    pub(crate) fn observe_dialog(&mut self, dialog: &Dialog) -> bool {
        let muted = dialog_notifications_muted(dialog);
        self.set_chat_muted(dialog.chat().id(), muted);
        muted
    }

    /// Returns whether the message's chat is currently known to be muted.
    pub(crate) fn message_chat_muted(&self, message: &Message) -> bool {
        self.muted_chat_ids.contains(&message.chat().id())
    }

    /// Applies a raw Telegram notification-settings update when it targets one concrete peer.
    pub(crate) fn apply_raw_update(&mut self, update: &tl::enums::Update) {
        let tl::enums::Update::NotifySettings(update) = update else {
            return;
        };
        let Some(chat_id) = notify_peer_chat_id(&update.peer) else {
            return;
        };
        let muted = peer_notify_settings_muted_at(&update.notify_settings, now_unix_seconds());
        self.set_chat_muted(chat_id, muted);
    }

    fn set_chat_muted(&mut self, chat_id: i64, muted: bool) {
        if muted {
            self.muted_chat_ids.insert(chat_id);
        } else {
            self.muted_chat_ids.remove(&chat_id);
        }
    }
}

fn dialog_is_muted_at(dialog: &Dialog, now_seconds: i64) -> bool {
    match &dialog.raw {
        tl::enums::Dialog::Dialog(dialog) => {
            peer_notify_settings_muted_at(&dialog.notify_settings, now_seconds)
        }
        tl::enums::Dialog::Folder(_) => false,
    }
}

fn peer_notify_settings_muted_at(
    settings: &tl::enums::PeerNotifySettings,
    now_seconds: i64,
) -> bool {
    match settings {
        tl::enums::PeerNotifySettings::Settings(settings) => settings
            .mute_until
            .is_some_and(|mute_until| i64::from(mute_until) > now_seconds),
    }
}

fn notify_peer_chat_id(peer: &tl::enums::NotifyPeer) -> Option<i64> {
    match peer {
        tl::enums::NotifyPeer::Peer(peer) => peer_chat_id(&peer.peer),
        tl::enums::NotifyPeer::NotifyForumTopic(topic) => peer_chat_id(&topic.peer),
        tl::enums::NotifyPeer::NotifyUsers
        | tl::enums::NotifyPeer::NotifyChats
        | tl::enums::NotifyPeer::NotifyBroadcasts => None,
    }
}

fn peer_chat_id(peer: &tl::enums::Peer) -> Option<i64> {
    match peer {
        tl::enums::Peer::User(peer) => Some(peer.user_id),
        tl::enums::Peer::Chat(peer) => Some(peer.chat_id),
        tl::enums::Peer::Channel(peer) => Some(peer.channel_id),
    }
}

fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(mute_until: Option<i32>) -> tl::enums::PeerNotifySettings {
        tl::types::PeerNotifySettings {
            show_previews: None,
            silent: None,
            mute_until,
            ios_sound: None,
            android_sound: None,
            other_sound: None,
            stories_muted: None,
            stories_hide_sender: None,
            stories_ios_sound: None,
            stories_android_sound: None,
            stories_other_sound: None,
        }
        .into()
    }

    #[test]
    fn future_mute_until_counts_as_muted() {
        let now = 1_000;

        assert!(peer_notify_settings_muted_at(&settings(Some(1_001)), now));
        assert!(!peer_notify_settings_muted_at(&settings(Some(999)), now));
        assert!(!peer_notify_settings_muted_at(&settings(Some(0)), now));
        assert!(!peer_notify_settings_muted_at(&settings(None), now));
    }

    #[test]
    fn notify_peer_extracts_concrete_chat_ids() {
        let user = tl::types::NotifyPeer {
            peer: tl::types::PeerUser { user_id: 42 }.into(),
        };
        let group = tl::types::NotifyPeer {
            peer: tl::types::PeerChat { chat_id: 43 }.into(),
        };
        let channel = tl::types::NotifyPeer {
            peer: tl::types::PeerChannel { channel_id: 44 }.into(),
        };

        assert_eq!(notify_peer_chat_id(&user.into()), Some(42));
        assert_eq!(notify_peer_chat_id(&group.into()), Some(43));
        assert_eq!(notify_peer_chat_id(&channel.into()), Some(44));
        assert_eq!(
            notify_peer_chat_id(&tl::enums::NotifyPeer::NotifyBroadcasts),
            None
        );
    }

    #[test]
    fn notify_settings_updates_refresh_cache() {
        let mut cache = NotificationMuteCache::default();
        let peer: tl::enums::NotifyPeer = tl::types::NotifyPeer {
            peer: tl::types::PeerUser { user_id: 42 }.into(),
        }
        .into();

        let muted_update = tl::types::UpdateNotifySettings {
            peer: peer.clone(),
            notify_settings: settings(Some(i32::MAX)),
        }
        .into();
        cache.apply_raw_update(&muted_update);

        assert!(cache.muted_chat_ids.contains(&42));

        let unmuted_update = tl::types::UpdateNotifySettings {
            peer,
            notify_settings: settings(Some(0)),
        }
        .into();
        cache.apply_raw_update(&unmuted_update);

        assert!(!cache.muted_chat_ids.contains(&42));
    }
}

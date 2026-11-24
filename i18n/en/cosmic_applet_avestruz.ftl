app-name = Avestruz

# Signing in
sign-in-prompt = Sign in with a Nostr key or a remote signer
sign-in-placeholder = nsec1…, npub1… or bunker://…
sign-in-help = A secret key (nsec) gives full access and is kept in the system keyring. A public key (npub) is read-only. A bunker address (bunker://) gives full access through a remote signer, where the key stays.
sign-in = Sign in
signing-in = Signing in…
sign-in-connecting = Connecting…
sign-in-waiting = Waiting for the remote signer. If it asks, approve the connection there.
sign-in-cancel = Cancel
sign-in-refused = The remote signer could not be connected to: { $error }
sign-in-client-key-help = If the signer only answers clients it knows, authorize this one there by its key, then sign in again. The key stays the same for this signer.
sign-in-secret-help = A bunker address with a secret in it is usually good for one use only: if it was used before, get a new one from the signer.
sign-in-copy-client-key = Copy client key
sign-in-client-key-copied = Copied
sign-out = Sign out
read-only = read-only

sign-in-error-empty = Enter a key to sign in.
sign-in-error-not-a-key = That is neither an nsec, an npub nor a bunker address.
sign-in-error-invalid-secret = That secret key (nsec) is not valid.
sign-in-error-invalid-public = That public key (npub) is not valid.
sign-in-error-invalid-bunker = That bunker address is not valid: it has to name a signer and a relay to reach it by.

# The feed
feed-mode-notes = Notes
feed-mode-replies = Replies
feed-look-further = Look further back
feed-new-note = Show 1 new note
feed-new-notes = Show { $count } new notes
feed-new-notes-key = Or press .
feed-nothing-recent = Nothing to show among the latest notes.
feed-following-nobody = You don't follow anyone yet.
feed-following-unknown = Finding out who you follow…
feed-refresh = Refresh
feed-search = Search
feed-notifications = Notifications
feed-notifications-new = { $count ->
    [one] One new notification
   *[other] { $count } new notifications
}
feed-compose = New note
feed-settings = Settings
feed-menu = Bookmarks, relays and settings
menu-bookmarks = Bookmarks
menu-relays = Relays
menu-media = Media servers
menu-keys = Keyboard (?)
feed-profile = Your profile
feed-loading = Loading notes…
feed-loading-older = Loading older notes…
feed-empty = No notes yet. Refresh to look again.
feed-end = That is everything the relays have.

# A note's actions
note-reply = Reply (r)
note-repost = Repost
note-repost-how = Repost or quote (R)
note-quote = Quote
note-like = Like
note-react = React
note-reacted = You reacted
note-react-more = More emoji
emoji-search-placeholder = Find an emoji
emoji-search-nothing = No emoji is called that.
note-show-more = Show more
note-show-less = Show less
note-zap = Zap (z)
note-zap-unavailable = No Lightning address to zap
note-seen-on = { $count ->
        [one] Seen on 1 relay
       *[other] Seen on { $count } relays
    }
note-share = Copy share link (y)
note-share-copied = Link copied
note-bookmark = Bookmark (b)
note-bookmark-remove = Remove bookmark (b)
note-bookmark-unreadable = Your bookmarks could not be read. { $error }
note-error-gone = That note is no longer at hand, so it was not reposted. Open its thread again and repost it from there.

# Threads
thread-title = Thread
thread-back = Back (h)
thread-looking = Looking for the note…
thread-not-found = That note could not be found on the relays.
thread-loading = Loading replies…
thread-no-replies = No replies yet.
thread-reply = Reply

# Profiles
profile-title = Profile
profile-back = Back (h)
profile-loading = Loading notes…
profile-no-notes = No notes yet.
profile-copy-npub = Copy npub
profile-npub-copied = Copied
profile-verified = This name is theirs: its domain says so
profile-follow = Follow
profile-unfollow = Unfollow
profile-follow-anyway = Follow anyway
profile-follow-no-list = None of your relays has a follow list of yours. Following { $name } starts a new one, which replaces any list you have elsewhere.
profile-follow-error = Your follow list was not changed: { $error }
profile-follow-unreachable = no relay could be reached

# Settings
settings-title = Settings
settings-back = Back (h)
settings-account = Account
settings-text-size = Text size
settings-text-size-description = How large notes are set.
settings-text-size-sample = Notes are set like this: the quick brown fox jumps over the lazy dog.
settings-keep = Notes kept
settings-keep-days = { $days ->
        [7] For a week
        [30] For a month
        [90] For three months
        [365] For a year
       *[other] For { $days } days
    }
settings-keep-forever = For good
settings-keep-description = How long other people's notes, and the reactions, reposts and zaps on them, stay on this machine once they were read. Older ones are asked of the relays again when they are looked at. Your own notes and your bookmarks stay.
settings-logging = Logging
settings-logging-description = How much Avestruz writes to the system journal. Applies immediately.

# What the keys do
keys-title = Keyboard
keys-back = Back (h)
keys-about = Getting about
keys-note = The marked note
keys-elsewhere = Elsewhere
keys-step = The next note, and the one before. It is marked, and put at the top.
keys-top = The very top of the list
keys-open = Open the marked note: its thread
keys-go-back = Back, as the arrow at the top left
keys-share = Copy a share link to the marked note
keys-play = Play the marked note's video, or hold it. If a link comes first in the note, open it
keys-reply = Reply to the marked note
keys-repost = Repost or quote the marked note: the arrows up and down choose which, and Enter does it
keys-zap = Zap the marked note
keys-bookmark = Bookmark the marked note, or take its bookmark off
keys-bookmarks = Show your bookmarks
keys-new = Show the new notes that wait
keys-help = Show this list
keys-publish = Publish what is being written
keys-leave-composer = Leave what is being written: at once if nothing is, and after asking whether to keep it as a draft if something is. In the text it rubs out first
keys-escape = Put a menu away, or close Avestruz
keys-copy-picture = On a picture: copy it
keys-right-click = Right click
keys-description = The keys work on any list of notes, and are the text's where something is being typed. To open Avestruz with a key from anywhere, bind one to “cosmic-applet-avestruz --toggle” in the desktop's Settings, under Keyboard, Shortcuts, Custom shortcuts.
picture-copied = Copied

# The composer
compose-title-note = New note
compose-title-reply = Reply
compose-title-quote = Quote
compose-back = Back
compose-placeholder-note = What's on your mind?
compose-placeholder-reply = Write a reply…
compose-placeholder-quote = Say something about it…
compose-count = { $count ->
        [one] { $count } character
       *[other] { $count } characters
    }
compose-shortcut = Ctrl+Enter
compose-publish = Publish
compose-reply = Reply
compose-publishing = Publishing…
compose-leaving = Keep what you wrote as a draft?
compose-leaving-keep = Keep it (Enter)
compose-leaving-discard = Discard it (Backspace)
compose-leaving-stay = Go on writing (Esc)
compose-attach = Attach a picture, a video or a sound
compose-attach-title = Attach to your note
compose-attach-kinds = Pictures, videos and sounds
compose-upload-preparing = Getting it ready…
compose-upload-sent = { $percent }%
compose-upload-cancel = Don't attach it
compose-error-parent-gone = The note you are replying to is no longer at hand. Open its thread again and reply from there.
compose-error-refused = No relay took it: { $reason }
compose-error-failed = It could not be sent: { $error }

# Zapping
zap-title = Zap
zap-back = Back (h)
zap-to = Zap { $name }
zap-sats = sats
zap-amount-placeholder = Amount
zap-comment-placeholder = Say something with it (optional)
zap-anonymous = You are not signed in with a key, so this zap will be anonymous.
zap-get-invoice = Get invoice
zap-requesting = Asking for an invoice…
zap-invoice-for = { $sats } sats for { $name }
zap-scan = Scan it with a Lightning wallet to pay.
zap-waiting = Waiting for the payment…
zap-not-seen = No payment was seen yet. If you paid, the zap will show under the note once the relays have it.
zap-plain-payment = { $name }'s wallet does not do zaps: this is a plain payment, and will not show under the note.
zap-no-code = This invoice is too long for a QR code. Copy it instead.
zap-copy = Copy invoice
zap-copied = Copied
zap-done = Done
zap-paid = Zapped { $sats } sats to { $name }
zap-error-address = That Lightning address cannot be used.
zap-error-unreachable = The Lightning address did not answer: { $error }
zap-error-refused = The Lightning address refused: { $reason }
zap-error-range = It takes between { $min } and { $max } sats.
zap-error-wrong-amount = The invoice that came back was not for the amount asked, and was thrown away.

# Bookmarks
bookmark-no-list = None of your relays has a bookmark list of yours. Bookmark the note again to start a new one, which replaces any list you have elsewhere.
bookmark-error = Your bookmarks were not changed: { $error }
bookmark-unreachable = no relay could be reached
bookmark-unreadable = Your bookmarks could not be read, so they were not changed. { $error }

notifications-title = Notifications
notifications-back = Back (h)
notifications-tab-all = All
notifications-tab-mentions = Mentions
notifications-tab-reactions = Reactions
notifications-tab-zaps = Zaps
notifications-loading = Looking for what is new for you…
notifications-loading-older = Looking further back…
notifications-end = That is all the relays have.
notifications-failed = No relay could be asked. They are asked again in a minute.
notifications-empty = Nothing yet. Replies, mentions, reactions, reposts and zaps show here.
notifications-empty-mentions = Nobody has replied to you or mentioned you yet.
notifications-empty-reactions = Nobody has reacted to your notes or reposted them yet.
notifications-empty-zaps = No zaps to show. A zap shows once your Lightning address says it was paid.
notification-reply = Replied to you
notification-thread-reply = Replied in a thread you are in
notification-mention = Mentioned you
notification-quote = Quoted you
notification-reaction = reacted to your note
notification-repost = reposted your note
notification-zap = zapped you { $sats } sats
bookmarks-title = Bookmarks
bookmarks-back = Back (h)
bookmarks-reading = Reading your bookmarks…
bookmarks-loading = Looking up the notes…
bookmarks-none = No bookmarks yet. The ribbon under a note bookmarks it.
bookmarks-end = That is all of them.
bookmarks-hidden = { $count ->
        [one] 1 bookmark is not shown: your relays don't have it, or it is not a note.
       *[other] { $count } bookmarks are not shown: your relays don't have them, or they are not notes.
    }

# Errors
error-dismiss = Dismiss
error-storage = The local event store could not be opened: { $error }
error-feed = Could not load notes from the relays: { $error }
error-relays-incomplete = not all of your relays answered, and those that did have no such list of yours
error-no-relay = No relay could be reached. Refresh to try again.
error-keyring = The system keyring refused: { $error }
error-remote-signer = The remote signer refused, or did not answer: { $error }
error-cipher = It could not be encrypted or decrypted with your key: { $error }
error-open-link = The link could not be opened: { $error }

relays-title = Relays
relays-back = Back (h)
relays-save = Save
relays-saving = Saving…
relays-reading = Asking your relays for your lists…
relays-read-only = Signed in read-only: your relays show here, and can't be changed.
relays-outbox = Outbox relays
relays-outbox-description = Where your notes are kept. What you write is sent here, and others look here for it. One to three is plenty.
relays-inbox = Inbox relays
relays-inbox-description = Where replies, comments, likes and zaps meant for you are sent. Others write here to reach you.
relays-messages = Direct message relays
relays-messages-description = Where direct messages for you are sent. Anyone can read which they are, and should find one to three here. They should ask who is reading (NIP-42), so that only you get yours.
relays-search = Search relays
relays-search-description = The relays asked when you search. They have to support search (NIP-50).
relays-indexer = Indexer relays
relays-indexer-description = Relays that keep everyone's profile and relay list. Asked about the people your own relays don't know.
relays-trusted = Trusted relays
relays-trusted-description = Relays you trust with who you are. Kept in your list for the clients that go by it.
relays-blocked = Blocked relays
relays-blocked-description = Relays that are never connected to because a note or someone else's list names them.
relays-use-connected = connected
relays-use-disconnected = not connected
relays-use-traffic = { $down } down, { $up } up
relays-use-attempts = { $worked } of { $all } connections worked
relays-use-latency = { $ms } ms
relays-lacks-search = Does not say it supports search (NIP-50), so a search may find nothing here.
relays-lacks-auth = Does not say it asks who is reading (NIP-42), so anyone can see which messages wait for you here, though not read them.
relays-lacks-nip = Does not say it supports NIP-{ $nip }, which the relays of this list have to.
relays-restricted-writes = Says it takes notes only from some people, so what others send you here may be refused.
relays-payment-required = Says it has to be paid before it is used, so what others send you here may be refused.
relays-auth-to-write = Says whoever connects has to say who they are first (NIP-42), so clients that don't will not reach you here.
relays-auth-to-read = Says whoever connects has to say who they are first (NIP-42), so clients that don't will not see your notes here.
relays-up = Move up
relays-down = Move down
relays-none = None set.
relays-add-placeholder = relay.example.com
relays-add = Add
relays-remove = Remove
relays-not-a-relay = “{ $typed }” is not a relay's address.
relays-not-read = Your relay lists could not be read: { $error }
relays-list-unreadable = The private part of this list could not be read, so it can't be changed here. { $error }
relays-list-unread = The private part of this list was not read.
relays-no-list = None of your relays has a list of yours for “{ $list }”. Save again to write a first one, which replaces any such list you have elsewhere.
relays-error = Your relays were not saved: { $error }
relays-unreadable = One of your lists could not be read, so it was not changed. { $error }

search-back = Back (h)
search-placeholder = People, notes, a name@domain, or an address
search-go = Search
search-people = People
search-following = following
search-notes = Notes
search-hint = Search for people by name and for notes by what they say. A name@domain, an npub, a note or an nevent is opened; a relay's address (wss://…) shows what the relay has. Nothing you search for is kept.
search-reading-relays = Asking your relays which relays you search…
search-no-relays = You have no search relays set. Add one under Relays: it has to support search (NIP-50).
search-open-relays = Relays
search-searching = Searching…
search-nothing = Nothing found.
search-failed = The search did not go through: { $error }
search-no-such-name = Nobody goes by { $name } there, or the domain did not answer.
search-relay-people = A relay's address shows the notes it got last. Search for people by name.
search-relay-searches = searches (NIP-50)
search-relay-no-search = does not say it searches
follow-no-list-elsewhere = None of your relays has a contact list of yours. Open their profile to follow them all the same.
search-secret = That is a secret key. It was not sent anywhere, and has been cleared.

# Videos
video-play = Play (Space)
video-pause = Pause (Space)
video-mute = Mute
video-unmute = Unmute
video-speed = Playback speed
video-open = Open in the browser
video-loading = Loading…
video-failed = This video can't be played here. { $error }
video-offer = Video
video-ytdlp-missing = Playing a YouTube video takes yt-dlp, which is not installed.
video-ytdlp-slow = yt-dlp did not answer in time.
video-ytdlp-runtime = yt-dlp has no JavaScript runtime to read YouTube with. Its wiki says how to give it one: github.com/yt-dlp/yt-dlp/wiki/EJS
video-ytdlp-failed = yt-dlp says: { $error }

# Media servers
media-title = Media servers
media-back = Back (h)
media-save = Save
media-saving = Saving…
media-reading = Asking your relays for your list…
media-read-only = Signed in read-only: your media servers show here, and can't be changed.
media-description = Where the pictures, videos and sounds you attach to a note are kept (Blossom). A file is sent to the first server that takes it, and the others fetch a copy from there, so that it is still to be found when one of them is gone.
media-none = None set. Add one to attach files to your notes.
media-first = Files are sent here first
media-copy = Fetches a copy
media-up = Move up
media-down = Move down
media-remove = Remove
media-add-placeholder = cdn.example.com
media-add = Add
media-privacy = Before a JPEG, a PNG or a WebP is sent, everything in the file that is not the picture is left out: where and when it was taken, with what, and by whom. Other files, videos among them, are sent as they are.
media-needed = Add a media server to attach files to your notes.
media-not-a-server = “{ $typed }” is not a media server's address. It has to be an https one.
media-not-read = Your media server list could not be read: { $error }
media-no-list = None of your relays has a media server list of yours. Save again to write a first one, which replaces any such list you have elsewhere.
media-error = Your media servers were not saved: { $error }

# Attaching
attach-unreachable = could not be reached ({ $error }).
attach-refused = said no ({ $status }): { $reason }.
attach-unreadable-answer = took it, and what it said of where it is could not be read ({ $error }).
attach-error-chooser = The file chooser could not be opened: { $error }
attach-error-read = { $name } could not be read: { $error }
attach-error-empty = { $name } is empty.
attach-error-too-large = { $name } is { $size }, and the most that is attached is { $most }.
attach-error-unreadable = { $name } is not put together as a picture of its kind is, so what it says beside the picture could not be left out. It was not sent.
attach-error-damaged = { $name } did not come through having what it says beside the picture left out. It was not sent.
attach-error-sign = { $name } was not sent. { $error }
attach-error-refused = No media server took { $name }. { $servers }

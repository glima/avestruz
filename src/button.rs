// SPDX-License-Identifier: MIT

//! The button in the panel, and the line between it and the window it
//! opens.
//!
//! Avestruz runs as two processes of the one program. The panel starts the
//! first, which is this: an applet that is a button and nothing else. It
//! starts the second, the same program told to be the window
//! ([`WINDOW_FLAG`]), which is the whole client (see [`crate::app`]).
//!
//! Two, because of who draws what. An applet's surfaces are the panel's:
//! the panel is a compositor of its own, takes what its applets draw and
//! hands it on, and takes no GPU buffers from them. An applet is therefore
//! drawn by the CPU, which is plenty for a button and was the dearest
//! thing about a list of pictures being scrolled. The window is no
//! applet: it puts a surface of its own on the session's compositor, and
//! the GPU draws it.
//!
//! ```mermaid
//! flowchart LR
//!     subgraph panel["The panel, a compositor of its own"]
//!         button["The button: an applet, drawn by the CPU"]
//!     end
//!     window["The window: the client, drawn by the GPU"]
//!     compositor["The session's compositor"]
//!     button -->|"starts it, and says: toggle"| window
//!     window -->|"says: unread 1, unread 0"| button
//!     button -. "its surface is the panel's" .-> panel
//!     panel --> compositor
//!     window -->|"a surface of its own"| compositor
//! ```
//!
//! They talk over the window's standard input and output, a line at a
//! time. The button says [`TOGGLE`] when it is pressed. The window says
//! [`UNREAD`] and `1` or `0`: whether the account has been told of
//! something it has not looked at, which the button shows as a dot.
//! Nothing else goes over the line: the window logs to its standard
//! error, which is the button's, which the panel hands to the journal.
//!
//! The window lives as long as the button does: when its input ends, it
//! leaves. If it goes first, the button starts another.
//!
//! The button is not the only one that can say [`TOGGLE`]. The window
//! listens at a socket in the session's runtime directory as well
//! ([`socket_path`]), and the program started with [`TOGGLE_FLAG`] says
//! it there and leaves: which is what a keyboard shortcut of the
//! desktop's runs, to open and close the popup without the mouse. One
//! window listens there: this one, whenever no other does (see
//! [`from_outside`]).
//!
//! ```mermaid
//! flowchart LR
//!     key["A key the desktop was told of"] -->|"runs"| said["The program, with --toggle"]
//!     said -->|"toggle, at the socket"| window["The window"]
//!     button["The button, pressed"] -->|"toggle, down the line"| window
//!     window --> popup["The popup opens, or closes"]
//! ```

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;

use cosmic::iced::widget::Stack;
use cosmic::iced::{Length, Subscription};
use cosmic::prelude::*;
use cosmic::widget::container;
use futures_util::{SinkExt, Stream};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use crate::views::components::{app_icon, dot_badge};

/// What the program is started with to be the window rather than the
/// button.
pub const WINDOW_FLAG: &str = "--window";

/// What the button says to the window when it is pressed.
pub const TOGGLE: &str = "toggle";

/// What the window says to the button, before `1` or `0`: whether there
/// is something the account has not looked at.
pub const UNREAD: &str = "unread";

/// The variable by which iced is told what to draw with, and what the
/// button is drawn with: the CPU renderer.
const RENDERER: &str = "ICED_BACKEND";
const CPU_RENDERER: &str = "tiny-skia";

/// What [`RENDERER`] said when the program started, before the button
/// took it for itself: the window's to go by.
static RENDERER_ASKED_FOR: OnceLock<Option<String>> = OnceLock::new();

/// How long after the window went the button starts another.
const RESTART_AFTER: Duration = Duration::from_secs(2);

/// How far in from the corner of the panel button the dot is that says
/// there are new notifications.
const DOT_INSET: u16 = 3;

/// The button in the panel. See the [module docs](self).
pub struct Button {
    core: cosmic::Core,
    /// The way to the window, once there is one.
    window: Option<mpsc::Sender<()>>,
    /// Whether the window said there is something not looked at.
    unread: bool,
}

/// What happens to the button.
#[derive(Debug, Clone)]
pub enum Message {
    /// It was pressed
    Pressed,
    /// A window was started: the way to it
    Linked(mpsc::Sender<()>),
    /// The window said whether there is something not looked at
    Unread(bool),
}

impl cosmic::Application for Button {
    type Executor = cosmic::executor::Default;
    type Flags = ();
    type Message = Message;

    const APP_ID: &'static str = crate::app::APP_ID;

    fn core(&self) -> &cosmic::Core {
        &self.core
    }

    fn core_mut(&mut self) -> &mut cosmic::Core {
        &mut self.core
    }

    fn init(core: cosmic::Core, _flags: Self::Flags) -> (Self, Task<cosmic::Action<Self::Message>>) {
        (Self { core, window: None, unread: false }, Task::none())
    }

    /// The app icon, sized and padded as the panel asks, with a dot over
    /// its corner while there is something not looked at. The dot takes no
    /// room: the button is the same size with it and without.
    fn view(&self) -> Element<'_, Self::Message> {
        let button = self.core.applet.icon_button_from_handle(app_icon()).on_press_down(Message::Pressed);
        if !self.unread {
            return button.into();
        }
        let dot = container(dot_badge()).align_right(Length::Fill).padding(DOT_INSET);
        Stack::new().push(button).push(dot).into()
    }

    fn update(&mut self, message: Self::Message) -> Task<cosmic::Action<Self::Message>> {
        match message {
            Message::Pressed => {
                // A press while no window is there, or while it is busy
                // with the last one, is lost: the next one is not.
                if let Some(window) = &self.window {
                    let _ = window.try_send(());
                }
            }
            Message::Linked(window) => self.window = Some(window),
            Message::Unread(unread) => self.unread = unread,
        }
        Task::none()
    }

    fn subscription(&self) -> Subscription<Self::Message> {
        Subscription::run(window)
    }

    fn style(&self) -> Option<cosmic::iced::theme::Style> {
        Some(cosmic::applet::style())
    }
}

/// Run the program as the button: an applet, drawn by the CPU whatever
/// the program was built to draw with.
pub fn run() -> cosmic::iced::Result {
    let _ = RENDERER_ASKED_FOR.set(std::env::var(RENDERER).ok());
    // SAFETY: `main` has started no thread yet when it comes here, so
    // nothing reads the environment while it is written.
    unsafe { std::env::set_var(RENDERER, CPU_RENDERER) };
    cosmic::applet::run::<Button>(())
}

/// Start the program again as the window, on the session's compositor
/// rather than the panel's: without the socket the panel gave the button,
/// and drawn with what was asked for, which by default is the GPU.
fn start_window() -> std::io::Result<Child> {
    let mut window = Command::new(std::env::current_exe()?);
    window.arg(WINDOW_FLAG).env_remove("WAYLAND_SOCKET").env_remove("X_PRIVILEGED_WAYLAND_SOCKET");
    match RENDERER_ASKED_FOR.get().cloned().flatten() {
        Some(renderer) => window.env(RENDERER, renderer),
        None => window.env_remove(RENDERER),
    };
    window.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit()).kill_on_drop(true).spawn()
}

/// What `line`, said by the window, means: whether there is something not
/// looked at, if that is what it says.
fn unread_said(line: &str) -> Option<bool> {
    match line.trim().strip_prefix(UNREAD)?.trim() {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    }
}

/// The window, for as long as the button is there: started, told of each
/// press, listened to, and started again if it goes.
fn window() -> impl Stream<Item = Message> {
    cosmic::iced::stream::channel(8, async |mut output| {
        loop {
            let (presses, mut pressed) = mpsc::channel(1);
            if output.send(Message::Linked(presses)).await.is_err() {
                return;
            }
            match start_window() {
                Ok(mut window) => {
                    let (Some(mut said_to), Some(said_by)) = (window.stdin.take(), window.stdout.take()) else {
                        return;
                    };
                    let mut said_by = BufReader::new(said_by).lines();
                    loop {
                        tokio::select! {
                            press = pressed.recv() => {
                                if press.is_none() {
                                    return;
                                }
                                let told = said_to.write_all(format!("{TOGGLE}\n").as_bytes()).await;
                                if told.is_err() || said_to.flush().await.is_err() {
                                    break;
                                }
                            }
                            line = said_by.next_line() => match line {
                                Ok(Some(line)) => {
                                    if let Some(unread) = unread_said(&line) {
                                        let _ = output.send(Message::Unread(unread)).await;
                                    }
                                }
                                _ => break,
                            },
                        }
                    }
                    tracing::warn!("the window went: another is started");
                    let _ = window.kill().await;
                }
                Err(e) => tracing::error!("the window could not be started: {e}"),
            }
            let _ = output.send(Message::Unread(false)).await;
            tokio::time::sleep(RESTART_AFTER).await;
        }
    })
}

/// What the program is started with to say [`TOGGLE`] to the window that
/// is running, and leave.
pub const TOGGLE_FLAG: &str = "--toggle";

/// What the socket the window listens at is called, in the session's
/// runtime directory.
const SOCKET_NAME: &str = "cosmic-applet-avestruz.sock";

/// How long whoever connects to the socket gets to say something.
const SAID_WITHIN: Duration = Duration::from_secs(1);

/// How often the window looks whether the socket is still its own to
/// listen at, and whether it has come to be nobody's.
const LOOKED_AT_EVERY: Duration = Duration::from_secs(2);

/// Where the window listens for what is said to it from outside: in the
/// session's runtime directory, which is its user's alone. `None` if the
/// session has none.
pub fn socket_path() -> Option<PathBuf> {
    dirs::runtime_dir().map(|directory| directory.join(SOCKET_NAME))
}

/// Say [`TOGGLE`] to the window that listens at `path`. An error if none
/// does.
pub fn say_toggle(path: &Path) -> std::io::Result<()> {
    let mut window = std::os::unix::net::UnixStream::connect(path)?;
    writeln!(window, "{TOGGLE}")
}

/// Which file is at `path`, as the file system tells files apart. `None`
/// if none is.
fn file_at(path: &Path) -> Option<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path).ok().map(|file| (file.dev(), file.ino()))
}

/// A socket the window listens at, and which file it was when the window
/// began to: the path may come to name another's socket, or nothing.
struct Listening {
    listener: tokio::net::UnixListener,
    file: Option<(u64, u64)>,
}

impl Listening {
    /// Listen at `path`, unless someone answers there: another window,
    /// which is left to. What is there and answers to nobody is what a
    /// window that is gone left behind, and is taken away first.
    fn at(path: &Path) -> std::io::Result<Option<Self>> {
        if std::os::unix::net::UnixStream::connect(path).is_ok() {
            return Ok(None);
        }
        let _ = std::fs::remove_file(path);
        let listener = tokio::net::UnixListener::bind(path)?;
        Ok(Some(Self { listener, file: file_at(path) }))
    }

    /// Whether `path` still names the socket listened at: not if the
    /// file was taken away, or another put in its place.
    fn is_still_at(&self, path: &Path) -> bool {
        self.file.is_some() && self.file == file_at(path)
    }
}

/// Whoever connects next to the socket listened at. Nobody, ever, while
/// none is; `None` if they could not be let in.
async fn next_at(listening: Option<&Listening>) -> Option<tokio::net::UnixStream> {
    match listening {
        Some(listening) => listening.listener.accept().await.ok().map(|(said_by, _)| said_by),
        None => std::future::pending().await,
    }
}

/// What is said to the window from outside, at the socket `path`, as the
/// window hears it: each `toggle`. Nothing, ever, if there is no path.
///
/// One window listens there, and it is this one whenever no other does.
/// It looks every [`LOOKED_AT_EVERY`]: a window that finds another
/// listening leaves the socket to it, and takes it once that one is gone;
/// one whose socket file was taken away from under it, or replaced by
/// one that answers to nobody, listens anew. So the socket is never left
/// with a window running and none hearing, whatever else was started and
/// stopped beside it, and whatever was deleted.
///
/// Nothing is taken away when the window leaves. A socket file with
/// nobody behind it is in nobody's way: the next window replaces it. And
/// taking it away is how a window that leaves late would take away the
/// socket of the one started after it.
pub fn from_outside<Message>(path: Option<PathBuf>, toggle: Message) -> impl Stream<Item = Message>
where
    Message: Clone + Send + 'static,
{
    heard_at(path, toggle, LOOKED_AT_EVERY)
}

/// [`from_outside`], looking at the socket every `looked_at_every`.
fn heard_at<Message>(path: Option<PathBuf>, toggle: Message, looked_at_every: Duration) -> impl Stream<Item = Message>
where
    Message: Clone + Send + 'static,
{
    cosmic::iced::stream::channel(4, async move |mut output| {
        let Some(path) = path else {
            return std::future::pending::<()>().await;
        };
        let mut listening: Option<Listening> = None;
        // Said once, not at every look.
        let mut refused = false;
        let mut look = tokio::time::interval(looked_at_every);
        look.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let said_by = tokio::select! {
                _ = look.tick() => {
                    if !listening.as_ref().is_some_and(|listening| listening.is_still_at(&path)) {
                        listening = match Listening::at(&path) {
                            Ok(listening) => {
                                refused = false;
                                listening
                            }
                            Err(e) => {
                                if !refused {
                                    tracing::warn!("nothing can be said to the window at {}: {e}", path.display());
                                }
                                refused = true;
                                None
                            }
                        };
                    }
                    continue;
                }
                said_by = next_at(listening.as_ref()) => said_by,
            };
            let Some(said_by) = said_by else {
                tokio::time::sleep(SAID_WITHIN).await;
                continue;
            };
            // One line from each, and not waited for long: whoever says
            // nothing does not keep the next from being heard.
            let mut said = BufReader::new(said_by).lines();
            let line = tokio::time::timeout(SAID_WITHIN, said.next_line()).await;
            if let Ok(Ok(Some(line))) = line
                && line.trim() == TOGGLE
                && output.send(toggle.clone()).await.is_err()
            {
                return;
            }
        }
    })
}

/// Say to the button, from the window, whether there is something not
/// looked at. To nobody, if the window was started by hand.
pub fn tell_unread(unread: bool) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{UNREAD} {}", u8::from(unread));
    let _ = out.flush();
}

/// What the button says to the window, as the window hears it: each
/// `toggle`, and that the button is gone when its end of the line closes.
pub fn from_button<Message>(toggle: Message, gone: Message) -> impl Stream<Item = Message>
where
    Message: Clone + Send + 'static,
{
    cosmic::iced::stream::channel(4, async move |mut output| {
        let mut said = BufReader::new(tokio::io::stdin()).lines();
        while let Ok(Some(line)) = said.next_line().await {
            if line.trim() == TOGGLE && output.send(toggle.clone()).await.is_err() {
                return;
            }
        }
        let _ = output.send(gone).await;
        // Said once: the stream stays open, so that it is not begun again.
        std::future::pending::<()>().await;
    })
}

#[cfg(test)]
mod tests {
    use super::{TOGGLE, UNREAD, from_outside, heard_at, say_toggle, unread_said};
    use futures_util::StreamExt;
    use std::time::Duration;

    /// A socket path of this test's own.
    fn socket_for(test: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("avestruz-test-{test}-{}.sock", std::process::id()))
    }

    /// Say toggle at `path`, as soon as someone listens there.
    async fn say_toggle_once_heard(path: &std::path::Path) {
        let said = async {
            while say_toggle(path).is_err() {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), said).await.expect("nobody came to listen");
    }

    /// A window that listens at `path` and looks at it every [`SOON`]:
    /// what it hears, and the task it runs as, which ends when aborted.
    fn window(
        path: &std::path::Path,
        name: &'static str,
    ) -> (tokio::sync::mpsc::UnboundedReceiver<&'static str>, tokio::task::JoinHandle<()>) {
        let (hears, heard) = tokio::sync::mpsc::unbounded_channel();
        let mut stream = Box::pin(heard_at(Some(path.to_path_buf()), name, SOON));
        let task = tokio::spawn(async move {
            while let Some(said) = stream.next().await {
                let _ = hears.send(said);
            }
        });
        (heard, task)
    }

    /// How often the windows of these tests look at their socket.
    const SOON: Duration = Duration::from_millis(20);

    /// What `heard` hears next, if it comes soon enough.
    async fn next(heard: &mut tokio::sync::mpsc::UnboundedReceiver<&'static str>) -> Option<&'static str> {
        tokio::time::timeout(Duration::from_secs(10), heard.recv()).await.ok().flatten()
    }

    #[test]
    fn toggle_said_at_the_socket_is_heard_by_the_window_and_nothing_else_is() {
        let path = socket_for("heard");
        // What a window that is gone left there is in nobody's way.
        std::fs::write(&path, "left behind").unwrap();
        assert!(say_toggle(&path).is_err(), "nobody listens there yet");

        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let (mut heard, task) = window(&path, "toggle heard");
            // Once the window listens: a toggle, something that is no
            // toggle, and someone who says nothing at all.
            say_toggle_once_heard(&path).await;
            {
                use std::io::Write;
                let mut other = std::os::unix::net::UnixStream::connect(&path).unwrap();
                writeln!(other, "something else").unwrap();
                drop(std::os::unix::net::UnixStream::connect(&path).unwrap());
            }
            assert_eq!(next(&mut heard).await, Some("toggle heard"));
            // And once only.
            assert!(tokio::time::timeout(Duration::from_millis(200), heard.recv()).await.is_err());
            task.abort();
        });
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_window_leaves_the_socket_to_the_one_that_listens_and_takes_it_when_that_one_is_gone() {
        let path = socket_for("two");
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let (mut first_heard, first) = window(&path, "first");
            say_toggle_once_heard(&path).await;
            assert_eq!(next(&mut first_heard).await, Some("first"));

            // Another is started beside it, and has looked a few times:
            // the first is the one that hears still.
            let (mut second_heard, second) = window(&path, "second");
            tokio::time::sleep(SOON * 5).await;
            say_toggle(&path).unwrap();
            assert_eq!(next(&mut first_heard).await, Some("first"));
            assert!(second_heard.try_recv().is_err());

            // The first goes, and leaves its socket file behind with
            // nobody at it: the second takes its place.
            first.abort();
            let _ = first.await;
            assert!(path.exists());
            say_toggle_once_heard(&path).await;
            assert_eq!(next(&mut second_heard).await, Some("second"));
            second.abort();
        });
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_window_whose_socket_file_was_taken_away_listens_anew() {
        let path = socket_for("taken");
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let (mut heard, task) = window(&path, "heard");
            say_toggle_once_heard(&path).await;
            assert_eq!(next(&mut heard).await, Some("heard"));

            // Deleted from under it: what something run beside it did.
            std::fs::remove_file(&path).unwrap();
            assert!(say_toggle(&path).is_err());
            say_toggle_once_heard(&path).await;
            assert_eq!(next(&mut heard).await, Some("heard"));

            // And replaced by a file that answers to nobody.
            std::fs::remove_file(&path).unwrap();
            std::fs::write(&path, "another's, and nobody at it").unwrap();
            say_toggle_once_heard(&path).await;
            assert_eq!(next(&mut heard).await, Some("heard"));
            task.abort();
        });
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn with_no_socket_to_listen_at_nothing_is_heard() {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async {
            let mut heard = Box::pin(from_outside(None, "toggle heard"));
            assert!(tokio::time::timeout(Duration::from_millis(100), heard.next()).await.is_err());
        });
    }

    #[test]
    fn the_window_says_whether_there_is_something_not_looked_at() {
        assert_eq!(unread_said(&format!("{UNREAD} 1")), Some(true));
        assert_eq!(unread_said(&format!("{UNREAD} 0\n")), Some(false));
        // Anything else on the line is not for the button.
        assert_eq!(unread_said(&format!("{UNREAD} many")), None);
        assert_eq!(unread_said(TOGGLE), None);
        assert_eq!(unread_said(""), None);
    }
}

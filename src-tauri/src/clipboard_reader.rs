//! Windows capture reads through an OLE data object. Office can render into an
//! STGMEDIUM without our holding OpenClipboard across its rendering callback.
//! Non-OLE producers are supported by Windows' default IDataObject wrapper.
//! Do not retain a reader across capture attempts or clipboard sequence changes.
//!
//! Reads run on their own apartment thread. GetClipboardData gives up on a hung
//! owner after 30 s, but GetData into a live OLE source waits for as long as
//! that source is frozen, and on an STA neither CoCancelCall nor a message
//! filter interrupts it. The capture thread stops waiting once no read has
//! finished for the same 30 s, and leaves that thread to finish, or to fail
//! when its owner exits.

use std::{
    marker::PhantomData,
    rc::Rc,
    sync::{mpsc, Mutex, PoisonError},
    time::Duration,
};
use windows::Win32::{
    Foundation::{GetLastError, SetLastError, ERROR_SUCCESS, LPARAM, WPARAM},
    System::{
        Com::{IDataObject, DVASPECT_CONTENT, FORMATETC, STGMEDIUM, TYMED_HGLOBAL},
        DataExchange::{
            CountClipboardFormats, GetClipboardSequenceNumber, IsClipboardFormatAvailable,
        },
        Memory::{GlobalLock, GlobalSize, GlobalUnlock},
        Ole::{OleGetClipboard, OleInitialize, OleUninitialize, ReleaseStgMedium},
        Threading::GetCurrentThreadId,
    },
    UI::WindowsAndMessaging::{
        DispatchMessageW, GetMessageW, PeekMessageW, PostThreadMessageW, TranslateMessage, MSG,
        PM_NOREMOVE, WM_APP,
    },
};

/// Windows' own limit for one delayed render requested through GetClipboardData.
/// The wait restarts whenever a read finishes, so a large copy that renders
/// several slow formats is not mistaken for a frozen owner.
const OWNER_TIMEOUT: Duration = Duration::from_secs(30);
const WM_READER_JOB: u32 = WM_APP + 1;

type Job = Box<dyn FnOnce() + Send>;
/// Called on the reader thread each time a read finishes.
type Progress = Box<dyn Fn()>;

enum Event<T> {
    Progress,
    Done(T),
}

struct Worker {
    jobs: mpsc::Sender<Job>,
    thread_id: u32,
}

static WORKER: Mutex<Option<Worker>> = Mutex::new(None);

pub(crate) enum ReadFailure {
    /// No read finished within [`OWNER_TIMEOUT`]. Retrying would wait that
    /// long again.
    Unresponsive,
    Failed(String),
}

/// Run `read` against a fresh [`Reader`] on the reader thread. Also returns
/// the clipboard sequence seen right after `read`, before the data object is
/// released: the caller needs to know whether every read came from one
/// generation, not whether a later copy has landed since.
pub(crate) fn with_reader<T: Send + 'static>(
    read: impl FnOnce(&Reader) -> T + Send + 'static,
) -> Result<(T, u32), ReadFailure> {
    on_reader_thread(
        move |progress| {
            Reader::new(progress).map(|reader| {
                let value = read(&reader);
                (value, unsafe { GetClipboardSequenceNumber() })
            })
        },
        OWNER_TIMEOUT,
    )?
    .map_err(ReadFailure::Failed)
}

fn on_reader_thread<T: Send + 'static>(
    job: impl FnOnce(Progress) -> T + Send + 'static,
    timeout: Duration,
) -> Result<T, ReadFailure> {
    let (events, received) = mpsc::channel();
    let job: Job = Box::new(move || {
        let progress = events.clone();
        let value = job(Box::new(move || {
            let _ = progress.send(Event::Progress);
        }));
        let _ = events.send(Event::Done(value));
    });
    // Held for the whole read: reads are serialized, and an abandoned worker
    // is never handed another job.
    let mut worker = WORKER.lock().unwrap_or_else(PoisonError::into_inner);
    if worker.is_none() {
        *worker = Some(spawn_worker().map_err(ReadFailure::Failed)?);
    }
    let current = worker.as_ref().expect("reader thread was just started");
    if current.jobs.send(job).is_err() {
        *worker = None;
        return Err(ReadFailure::Failed(
            "clipboard reader thread stopped".into(),
        ));
    }
    // The worker drains its queue after every job, so only an idle worker
    // needs this. Without it the job would wait out the timeout and be
    // blamed on the clipboard owner.
    if let Err(error) =
        unsafe { PostThreadMessageW(current.thread_id, WM_READER_JOB, WPARAM(0), LPARAM(0)) }
    {
        *worker = None;
        return Err(ReadFailure::Failed(format!(
            "could not wake clipboard reader thread: {error}"
        )));
    }
    loop {
        match received.recv_timeout(timeout) {
            Ok(Event::Done(value)) => return Ok(value),
            Ok(Event::Progress) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {
                *worker = None;
                return Err(ReadFailure::Unresponsive);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                *worker = None;
                return Err(ReadFailure::Failed(
                    "clipboard reader thread stopped".into(),
                ));
            }
        }
    }
}

fn spawn_worker() -> Result<Worker, String> {
    let (jobs, queue) = mpsc::channel::<Job>();
    let (ready, started) = mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("cubby-clipboard-reader".to_string())
        .spawn(move || {
            let apartment = match Apartment::new() {
                Ok(apartment) => apartment,
                Err(error) => {
                    let _ = ready.send(Err(error));
                    return;
                }
            };
            let mut msg = MSG::default();
            // Create the message queue before the capture thread posts to it.
            let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
            let _ = ready.send(Ok(unsafe { GetCurrentThreadId() }));
            'run: loop {
                loop {
                    match queue.try_recv() {
                        Ok(job) => job(),
                        Err(mpsc::TryRecvError::Empty) => break,
                        // Abandoned after a hung read, or the app is exiting.
                        Err(mpsc::TryRecvError::Disconnected) => break 'run,
                    }
                }
                // An STA owns COM and OLE windows: keep pumping while idle so
                // broadcasts and sent messages to them never block their sender.
                if unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 <= 0 {
                    break;
                }
                if msg.message != WM_READER_JOB {
                    unsafe {
                        let _ = TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            }
            drop(apartment);
        })
        .map_err(|e| e.to_string())?;
    let thread_id = started
        .recv()
        .map_err(|_| "clipboard reader thread exited during startup".to_string())??;
    Ok(Worker { jobs, thread_id })
}

/// Balanced initialization on the reader thread. Neither this guard nor its
/// data objects may move to a different COM apartment.
struct Apartment(PhantomData<Rc<()>>);
impl Apartment {
    fn new() -> Result<Self, String> {
        unsafe { OleInitialize(None) }.map_err(|e| e.to_string())?;
        Ok(Self(PhantomData))
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { OleUninitialize() };
    }
}

/// CountClipboardFormats does not require OpenClipboard. Clear last-error so an
/// API failure is never interpreted as an empty clipboard / password auto-clear.
pub(crate) fn is_empty() -> bool {
    unsafe {
        SetLastError(ERROR_SUCCESS);
        CountClipboardFormats() == 0 && GetLastError() == ERROR_SUCCESS
    }
}

pub(crate) fn available(format: u32) -> bool {
    unsafe { IsClipboardFormatAvailable(format) }.is_ok()
}

pub(crate) struct Reader {
    object: IDataObject,
    sequence: u32,
    progress: Progress,
    _apartment: PhantomData<Rc<()>>,
}
impl Reader {
    fn new(progress: Progress) -> Result<Self, String> {
        let sequence = unsafe { GetClipboardSequenceNumber() };
        Ok(Self {
            object: unsafe { OleGetClipboard() }.map_err(|e| e.to_string())?,
            sequence,
            progress,
            _apartment: PhantomData,
        })
    }

    pub(crate) fn bytes(&self, format: u32) -> Result<Vec<u8>, String> {
        // Availability is a lock-free hint. A failed advertised read is still
        // transient, and the caller binds the entire attempt to its sequence.
        if !available(format) {
            return Err("clipboard format unavailable".into());
        }
        let read = self.ole_bytes(format);
        (self.progress)();
        read.or_else(|error| {
            // A live OLE source answers GetData itself and can fail a read
            // that GetClipboardData still serves, for example by rejecting
            // calls while busy. Retry the way the previous reader did, but only
            // for this same generation: a moved sequence means the owner is
            // publishing, and Excel fails its copy if it finds the clipboard
            // open then.
            if unsafe { GetClipboardSequenceNumber() } != self.sequence || !available(format) {
                return Err(error);
            }
            let read =
                clipboard_win::get_clipboard::<Vec<u8>, _>(clipboard_win::formats::RawData(format));
            (self.progress)();
            read.map_err(|fallback| format!("{error}; GetClipboardData: {fallback}"))
        })
    }

    fn ole_bytes(&self, format: u32) -> Result<Vec<u8>, String> {
        let format = FORMATETC {
            cfFormat: u16::try_from(format).map_err(|e| e.to_string())?,
            ptd: std::ptr::null_mut(),
            dwAspect: DVASPECT_CONTENT.0,
            lindex: -1,
            tymed: TYMED_HGLOBAL.0 as u32,
        };
        let medium = Medium(unsafe { self.object.GetData(&format) }.map_err(|e| e.to_string())?);
        if medium.0.tymed != TYMED_HGLOBAL.0 as u32 {
            return Err("clipboard returned an unexpected storage medium".into());
        }
        let handle = unsafe { medium.0.u.hGlobal };
        let size = unsafe { GlobalSize(handle) };
        if size == 0 {
            return Err("clipboard returned an invalid or empty memory handle".into());
        }
        let data = unsafe { GlobalLock(handle) };
        if data.is_null() {
            return Err("could not lock clipboard storage medium".into());
        }
        // Only copy while the medium is locked. Decode, parse and compress
        // after releasing it. No caller sees borrowed producer memory.
        let result = unsafe { std::slice::from_raw_parts(data.cast::<u8>(), size) }.to_vec();
        let _ = unsafe { GlobalUnlock(handle) };
        Ok(result)
    }

    pub(crate) fn text(&self) -> Result<String, String> {
        Ok(decode_unicode(&self.bytes(13)?))
    }
}

struct Medium(STGMEDIUM);
impl Drop for Medium {
    fn drop(&mut self) {
        unsafe { ReleaseStgMedium(&mut self.0) };
    }
}

fn decode_unicode(bytes: &[u8]) -> String {
    // A producer's odd-sized allocation leaves a stray byte after the last
    // code unit. clipboard-win ignored it too; rejecting it lost the copy.
    let (pairs, _) = bytes.as_chunks::<2>();
    let units: Vec<u16> = pairs
        .iter()
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .take_while(|unit| *unit != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::{decode_unicode, on_reader_thread, ReadFailure};
    use std::time::Duration;

    #[test]
    fn a_read_stuck_past_the_timeout_is_abandoned_for_a_fresh_reader_thread() {
        let (stuck_on, stuck_thread) = std::sync::mpsc::channel();
        let stuck = on_reader_thread(
            move |_| {
                let _ = stuck_on.send(std::thread::current().id());
                std::thread::sleep(Duration::from_millis(500));
            },
            Duration::from_millis(50),
        );
        assert!(matches!(stuck, Err(ReadFailure::Unresponsive)));
        let stuck_thread = stuck_thread.recv().expect("the stuck read started");
        // That thread is still sleeping; only a new one can answer.
        let (thread, value) =
            on_reader_thread(|_| (std::thread::current().id(), 7), Duration::from_secs(5))
                .ok()
                .expect("a fresh reader thread serves the next read");
        assert_eq!(value, 7);
        assert_ne!(thread, stuck_thread);
        let (again, _) =
            on_reader_thread(|_| (std::thread::current().id(), 0), Duration::from_secs(5))
                .ok()
                .expect("the healthy reader thread is reused");
        assert_eq!(thread, again);
    }

    #[test]
    fn reads_that_keep_finishing_are_not_mistaken_for_a_frozen_owner() {
        // Eight slow formats take longer than the limit in total, but each one
        // finishes well within it.
        let finished = on_reader_thread(
            |progress| {
                for _ in 0..8 {
                    std::thread::sleep(Duration::from_millis(50));
                    progress();
                }
                "all formats"
            },
            Duration::from_millis(250),
        );
        assert!(matches!(finished, Ok("all formats")));
    }

    #[test]
    fn an_owner_that_stops_rendering_after_some_progress_still_times_out() {
        let stalled = on_reader_thread(
            |progress| {
                progress();
                progress();
                std::thread::sleep(Duration::from_millis(500));
            },
            Duration::from_millis(50),
        );
        assert!(matches!(stalled, Err(ReadFailure::Unresponsive)));
    }

    #[test]
    fn unicode_retains_whitespace_and_supplementary_characters() {
        let body = "  cell\t😀\r\n";
        let bytes: Vec<u8> = body
            .encode_utf16()
            .chain([0, 88])
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(decode_unicode(&bytes), body);
        assert_eq!(decode_unicode(&[0, 0]), "");
        assert_eq!(decode_unicode(&[b'a', 0, b'b', 0, 7]), "ab");
        assert_eq!(decode_unicode(&[1]), "");
    }
}

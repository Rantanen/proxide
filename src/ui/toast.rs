use std::cmp::{Ord, Ordering, PartialOrd};
use std::collections::BinaryHeap;
use std::sync::{
    mpsc::{channel, Receiver, Sender},
    Mutex, Once,
};
use std::time::{Duration, Instant};
use uuid::Uuid;

use lazy_static::lazy_static;

static ONCE: Once = Once::new();
lazy_static! {
    static ref CHANNEL: (Mutex<Sender<ToastEvent>>, Mutex<Receiver<ToastEvent>>) = {
        let (sender, receiver) = channel();
        (Mutex::new(sender), Mutex::new(receiver))
    };
    static ref TIMER_CHANNEL: (Mutex<Sender<FutureEvent>>, Mutex<Receiver<FutureEvent>>) = {
        let (sender, receiver) = channel();
        (Mutex::new(sender), Mutex::new(receiver))
    };
}

struct FutureEvent
{
    instant: Instant,
    event: ToastEvent,
}

impl Ord for FutureEvent
{
    fn cmp(&self, other: &Self) -> Ordering
    {
        // Reverse the comparison. The smallest instant should be the one
        // with the highest priority.
        other.instant.cmp(&self.instant)
    }
}

impl PartialOrd for FutureEvent
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering>
    {
        Some(self.cmp(other))
    }
}

impl Eq for FutureEvent {}

impl PartialEq for FutureEvent
{
    fn eq(&self, other: &Self) -> bool
    {
        // clippy reports an "unconditional recursion"false positive here in the pipeline with:
        // "self.instant.eq(&other.instant)"
        PartialEq::<Instant>::eq(&self.instant, &other.instant)
    }
}

#[derive(Debug)]
pub enum ToastEvent
{
    Show
    {
        uuid: Uuid,
        text: String,
        error: bool,
    },
    Close
    {
        uuid: Uuid
    },
}

pub fn recv() -> ToastEvent
{
    // Hold a lock on the receiver to allow only one thread to receive at a time.
    //
    // Ideally only one thread should be responsible for receiving these messages,
    // but who knows what's going to happen here.
    let receiver = CHANNEL.1.lock().expect("Mutex poisoned");
    receiver.recv().unwrap()
}

pub fn show_message<T: ToString>(text: T)
{
    show_toast(text.to_string(), false);
}

pub fn show_error<T: ToString>(text: T)
{
    // We'll want to log the errors into the log as well.
    let string = text.to_string();
    log::error!("{}", string);
    show_toast(string, true);
}

fn show_toast(text: String, error: bool)
{
    ensure_running();

    let uuid = Uuid::new_v4();
    let primary_sender = CHANNEL.0.lock().expect("Mutex poisoned").clone();
    let timer_sender = TIMER_CHANNEL.0.lock().expect("Mutex poisoned").clone();

    primary_sender
        .send(ToastEvent::Show { uuid, text, error })
        .unwrap();
    timer_sender
        .send(FutureEvent {
            instant: Instant::now() + Duration::from_secs(5),
            event: ToastEvent::Close { uuid },
        })
        .unwrap();
}

fn ensure_running()
{
    ONCE.call_once(|| {
        std::mem::forget(std::thread::spawn(|| {
            // This thread should hold the lock on the receiver all the time.
            // There's no need for anyone else to read these messages.
            let receiver = TIMER_CHANNEL.1.lock().expect("Mutex poisoned");
            let sender = CHANNEL.0.lock().expect("Mutex poisoned").clone();

            // Collection of future events to send.
            let mut events = BinaryHeap::new();

            // The outer loop handles the case where there are no existing events in the map.
            while let Ok(e) = receiver.recv() {
                let mut next_timeout = e.instant - Instant::now();
                events.push(e);

                // The inner loop is responsible for waiting for events to resolve.
                loop {
                    match receiver.recv_timeout(next_timeout) {
                        Ok(new_event) => {
                            // Received a new event before the timeout occurred.
                            // Push the event into the queue and resolve the next timeout again.
                            events.push(new_event);
                            next_timeout = events.peek().unwrap().instant - Instant::now();
                        }
                        Err(_) => {
                            // Timeout happened. Pop the next event and send it.
                            let next_event = events.pop().unwrap();
                            sender
                                .send(next_event.event)
                                .expect("Toast receiver has died");

                            // If there are moer events in the queue, wait for them to resolve.
                            // Otherwise exit the inner loop to go back to the outer one that waits for
                            // events without a timeout.
                            match events.peek() {
                                Some(e) => next_timeout = e.instant - Instant::now(),
                                None => break,
                            }
                        }
                    }
                }
            }
        }));
    });
}

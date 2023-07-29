use crate::interface::{DataIn, DataOut, Interface};
use chrono::Local;
use rand::rngs::ThreadRng;
use rand::Rng;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::thread::sleep;
use std::time::Duration;

pub struct LoopBackIF {
    if_tx: Sender<DataIn>,
    data_rx: Receiver<DataOut>,
    send_interval: Duration,
    is_connected: Arc<AtomicBool>,
}

impl Drop for LoopBackIF {
    fn drop(&mut self) {
        self.if_tx.send(DataIn::Exit).unwrap()
    }
}

impl Interface for LoopBackIF {
    fn is_connected(&self) -> bool {
        self.is_connected.load(Ordering::SeqCst)
    }

    fn send(&self, data: DataIn) {
        self.if_tx.send(data).unwrap()
    }

    fn try_recv(&self) -> Result<DataOut, TryRecvError> {
        self.data_rx.try_recv()
    }

    fn description(&self) -> String {
        format!("LoopBack {}ms", self.send_interval.as_millis())
    }
}

impl LoopBackIF {
    const RECONNECT_RATE: f32 = 0.5;
    const DISCONNECT_RATE: f32 = 0.0;
    const UPDATE_CONNECTION_INTERVAL: Duration = Duration::from_secs(2);

    pub fn new<F>(data_to_send: F, send_interval: Duration) -> Self
    where
        F: Fn() -> String,
        F: Send + Clone + 'static,
    {
        let (if_tx, if_rx) = channel();
        let (data_tx, data_rx) = channel();
        let data_tx2 = data_tx.clone();

        let is_connected = Arc::new(AtomicBool::new(false));
        let is_connected2 = is_connected.clone();
        let is_connected3 = is_connected.clone();
        let is_connected4 = is_connected.clone();

        thread::spawn(move || {
            LoopBackIF::task(if_rx, data_tx, is_connected2);
        });

        thread::spawn(move || loop {
            sleep(send_interval);
            if is_connected3.load(Ordering::SeqCst) {
                data_tx2
                    .send(DataOut::Data(Local::now(), data_to_send()))
                    .expect("Cannot forward message read from loopback");
            }
        });

        thread::spawn(move || loop {
            sleep(LoopBackIF::UPDATE_CONNECTION_INTERVAL);
            let mut rng = rand::thread_rng();
            if is_connected4.load(Ordering::SeqCst) {
                LoopBackIF::rng_disconnet(&mut rng, is_connected4.clone());
            } else {
                LoopBackIF::reconnect(&mut rng, is_connected4.clone());
            }
        });

        Self {
            if_tx,
            data_rx,
            send_interval,
            is_connected,
        }
    }

    fn reconnect(rng: &mut ThreadRng, is_connected: Arc<AtomicBool>) {
        if rng.gen::<f32>() <= LoopBackIF::RECONNECT_RATE {
            is_connected.store(true, Ordering::SeqCst);
        }
    }

    fn rng_disconnet(rng: &mut ThreadRng, is_connected: Arc<AtomicBool>) {
        if rng.gen::<f32>() <= LoopBackIF::DISCONNECT_RATE {
            is_connected.store(false, Ordering::SeqCst);
        }
    }

    fn task(if_rx: Receiver<DataIn>, data_tx: Sender<DataOut>, is_connected: Arc<AtomicBool>) {
        'task: loop {
            if let Ok(data_to_send) = if_rx.try_recv() {
                match data_to_send {
                    DataIn::Exit => break 'task,
                    DataIn::Data(data_to_send) => {
                        if is_connected.load(Ordering::SeqCst) {
                            data_tx
                                .send(DataOut::ConfirmData(Local::now(), data_to_send))
                                .expect("Cannot send data confirm");
                        } else {
                            data_tx
                                .send(DataOut::FailData(Local::now(), data_to_send))
                                .expect("Cannot send data fail");
                        }
                    }
                    DataIn::Command(command_name, data_to_send) => {
                        if is_connected.load(Ordering::SeqCst) {
                            data_tx
                                .send(DataOut::ConfirmCommand(
                                    Local::now(),
                                    command_name,
                                    data_to_send,
                                ))
                                .expect("Cannot send command confirm");
                        } else {
                            data_tx
                                .send(DataOut::FailCommand(
                                    Local::now(),
                                    command_name,
                                    data_to_send,
                                ))
                                .expect("Cannot send command fail");
                        }
                    }
                    DataIn::HexString(data) => {
                        if is_connected.load(Ordering::SeqCst) {
                            data_tx
                                .send(DataOut::ConfirmHexString(Local::now(), data))
                                .expect("Cannot send hex confirm")
                        } else {
                            data_tx
                                .send(DataOut::FailHexString(Local::now(), data))
                                .expect("Cannot send hex fail")
                        }
                    }
                }
            };
        }
    }
}

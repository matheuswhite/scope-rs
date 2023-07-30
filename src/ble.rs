use crate::interface::{DataIn, DataOut, Interface};
use btleplug::api::bleuuid::BleUuid;
use btleplug::api::{
    Central, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use chrono::Local;
use futures::stream::StreamExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tokio::{sync::Mutex, time::sleep};
use uuid::Uuid;

static IS_CONNECTED: AtomicBool = AtomicBool::new(false);

pub struct BleIF {
    ble_tx: Sender<DataIn>,
    data_rx: Receiver<DataOut>,
    tx_uuid: Uuid,
    rx_uuid: Uuid,
    dev_name: String,
}

impl Drop for BleIF {
    fn drop(&mut self) {
        self.ble_tx.send(DataIn::Exit).unwrap()
    }
}

impl Interface for BleIF {
    fn is_connected(&self) -> bool {
        IS_CONNECTED.load(Ordering::SeqCst)
    }

    fn send(&self, data: DataIn) {
        self.ble_tx.send(data).unwrap();
    }

    fn try_recv(&self) -> Result<DataOut, TryRecvError> {
        self.data_rx.try_recv()
    }

    fn description(&self) -> String {
        format!(
            "BLE [{} | ↑ {}, ↓ {}]",
            self.dev_name,
            self.tx_uuid.to_short_string(),
            self.rx_uuid.to_short_string(),
        )
    }
}

impl BleIF {
    pub fn new(tx_uuid: Uuid, rx_uuid: Uuid, dev_name: &str) -> Self {
        let (ble_tx, ble_rx) = channel();
        let (data_tx, data_rx) = channel();

        let dev_name2 = dev_name.to_owned();
        thread::spawn(move || {
            BleIF::entry(ble_rx, data_tx, tx_uuid, rx_uuid, dev_name2);
        });

        Self {
            ble_tx,
            data_rx,
            tx_uuid,
            rx_uuid,
            dev_name: dev_name.to_owned(),
        }
    }

    async fn search_device(adapter: &Adapter, dev_name: &str) -> Peripheral {
        'search_loop: loop {
            while let Err(_) = adapter.start_scan(ScanFilter::default()).await {
                sleep(Duration::from_millis(200)).await;
            }

            let peripherals = adapter.peripherals().await.unwrap();

            for p in peripherals {
                let local_name = p.properties().await.unwrap().unwrap().local_name;

                if local_name.iter().any(|name| name.contains(dev_name)) {
                    break 'search_loop p;
                }
            }

            sleep(Duration::from_millis(200)).await;
        }
    }

    async fn reconnect(
        adapter: &Adapter,
        tx_uuid: Uuid,
        rx_uuid: Uuid,
        dev_name: &str,
    ) -> (Peripheral, Characteristic) {
        loop {
            let device = BleIF::search_device(&adapter, dev_name).await;

            let Ok(_) = device.connect().await else {
                continue;
            };
            let Ok(_) = device.discover_services().await else {
                continue;
            };

            let chars = device.characteristics();
            let Some(tx_characterist) = chars.iter().find(|c| c.uuid == tx_uuid) else {
                continue;
            };
            let Some(rx_characterist) = chars.iter().find(|c| c.uuid == rx_uuid) else {
                continue;
            };

            let Ok(_) = device.subscribe(&rx_characterist).await else {
                continue;
            };

            break (device, tx_characterist.clone());
        }
    }

    async fn reconnect_task(
        adapter: Adapter,
        tx_uuid: Uuid,
        rx_uuid: Uuid,
        dev_name: String,
        device: Arc<Mutex<Peripheral>>,
        tx_char: Arc<Mutex<Characteristic>>,
    ) {
        loop {
            {
                let device_guard = device.lock().await;
                if !device_guard.is_connected().await.unwrap() {
                    IS_CONNECTED.store(false, Ordering::SeqCst);
                }
            }

            if !IS_CONNECTED.load(Ordering::SeqCst) {
                let (dev, tx_char_new) =
                    BleIF::reconnect(&adapter, tx_uuid, rx_uuid, &dev_name).await;

                {
                    let mut device_guard = device.lock().await;
                    *device_guard = dev;
                }

                {
                    let mut tx_char_gaurd = tx_char.lock().await;
                    *tx_char_gaurd = tx_char_new;
                }

                IS_CONNECTED.store(true, Ordering::SeqCst);
            }

            sleep(Duration::from_millis(200)).await;
        }
    }

    async fn write_task(
        ble_rx: Receiver<DataIn>,
        data_tx: Sender<DataOut>,
        device: Arc<Mutex<Peripheral>>,
        characteristic: Arc<Mutex<Characteristic>>,
    ) {
        'task: loop {
            let dev_guard = device.lock().await;
            let tx_char = characteristic.lock().await;

            let data_to_send = ble_rx.try_recv();

            if let Ok(data_to_send) = data_to_send {
                match data_to_send {
                    DataIn::Exit => {
                        // TODO not working
                        while let Err(_) = dev_guard.disconnect().await {}
                        break 'task;
                    }
                    DataIn::Data(data_to_send) => match dev_guard
                        .write(
                            &tx_char,
                            format!("{}\r\n", data_to_send).as_bytes(),
                            WriteType::WithoutResponse,
                        )
                        .await
                    {
                        Ok(_) => data_tx
                            .send(DataOut::ConfirmData(Local::now(), data_to_send))
                            .expect("Cannot send data confirm"),
                        Err(_) => {
                            data_tx
                                .send(DataOut::FailData(Local::now(), data_to_send))
                                .expect("Canot send data fail");
                        }
                    },
                    DataIn::Command(command_name, data_to_send) => match dev_guard
                        .write(
                            &tx_char,
                            format!("{}\r\n", data_to_send).as_bytes(),
                            WriteType::WithoutResponse,
                        )
                        .await
                    {
                        Ok(_) => data_tx
                            .send(DataOut::ConfirmCommand(
                                Local::now(),
                                command_name,
                                data_to_send,
                            ))
                            .expect("Cannot send command confirm"),
                        Err(_) => {
                            data_tx
                                .send(DataOut::FailCommand(
                                    Local::now(),
                                    command_name,
                                    data_to_send,
                                ))
                                .expect("Canot send command fail");
                        }
                    },
                    DataIn::HexString(bytes) => match dev_guard
                        .write(&tx_char, bytes.as_slice(), WriteType::WithoutResponse)
                        .await
                    {
                        Ok(_) => data_tx
                            .send(DataOut::ConfirmHexString(Local::now(), bytes))
                            .expect("Cannot send hex data confirm"),
                        Err(_) => {
                            data_tx
                                .send(DataOut::FailHexString(Local::now(), bytes))
                                .expect("Canot send hex data fail");
                        }
                    },
                    DataIn::File(idx, total, filename, content) => match dev_guard
                        .write(
                            &tx_char,
                            format!("{}\n", content).as_bytes(),
                            WriteType::WithoutResponse,
                        )
                        .await
                    {
                        Ok(_) => data_tx
                            .send(DataOut::ConfirmFile(
                                Local::now(),
                                idx,
                                total,
                                filename,
                                content,
                            ))
                            .expect("Cannot send file confirm"),
                        Err(_) => {
                            data_tx
                                .send(DataOut::FailFile(
                                    Local::now(),
                                    idx,
                                    total,
                                    filename,
                                    content,
                                ))
                                .expect("Canot send file fail");
                        }
                    },
                }
            }
        }
    }

    async fn read_task(data_tx: Sender<DataOut>, device: Arc<Mutex<Peripheral>>) {
        let mut line = String::new();

        loop {
            let mut notification_stream = {
                let dev_guard = device.lock().await;

                dev_guard.notifications().await.unwrap().take(1)
            };

            if let Some(notification) = notification_stream.next().await {
                line += std::str::from_utf8(&notification.value).unwrap();
                if line.contains('\n') {
                    data_tx
                        .send(DataOut::Data(Local::now(), line.clone()))
                        .expect("Cannot forward message read from ble");
                    line.clear();
                }
            }
        }
    }

    #[tokio::main]
    async fn entry(
        ble_rx: Receiver<DataIn>,
        data_tx: Sender<DataOut>,
        tx_uuid: Uuid,
        rx_uuid: Uuid,
        dev_name: String,
    ) {
        let manager = Manager::new().await.unwrap();
        let adapters = manager.adapters().await.unwrap();
        let adapter = adapters.into_iter().nth(0).unwrap();

        let (dev, tx_char) = BleIF::reconnect(&adapter, tx_uuid, rx_uuid, &dev_name).await;

        let device = Arc::new(Mutex::new(dev));
        let tx_char = Arc::new(Mutex::new(tx_char));

        let device2 = device.clone();
        let tx_char2 = tx_char.clone();
        let dev_name2 = dev_name.clone();
        tokio::spawn(BleIF::reconnect_task(
            adapter, tx_uuid, rx_uuid, dev_name2, device2, tx_char2,
        ));

        let data_tx2 = data_tx.clone();
        let device3 = device.clone();
        let write_task_handler =
            tokio::spawn(BleIF::write_task(ble_rx, data_tx2, device3, tx_char));

        tokio::spawn(BleIF::read_task(data_tx, device));

        write_task_handler.await.unwrap();
    }
}

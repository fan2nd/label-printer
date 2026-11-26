use btleplug::{
    api::{Central, CentralEvent, CentralState, Peripheral as _},
    platform::{Adapter, Peripheral, PeripheralId},
};
use futures::{future, stream::BoxStream, FutureExt, StreamExt};
use std::time::Duration;
use tokio::time::{sleep, timeout};

pub type DisconnectMonitor = future::BoxFuture<'static, ()>;
const STATUS_TIMEOUT: Duration = Duration::from_secs(3);

// CoreBluetooth does not answer is_connected for a peripheral it has already removed.
pub async fn connection_state(peripheral: &Peripheral) -> Option<bool> {
    match timeout(STATUS_TIMEOUT, peripheral.is_connected()).await {
        Ok(Ok(connected)) => Some(connected),
        Ok(Err(btleplug::Error::DeviceNotFound | btleplug::Error::NotConnected)) => Some(false),
        _ => None,
    }
}

async fn wait_for_disconnect_event(mut events: BoxStream<'_, CentralEvent>, id: &PeripheralId) {
    while let Some(event) = events.next().await {
        match event {
            CentralEvent::DeviceDisconnected(device) if device == *id => return,
            CentralEvent::StateUpdate(CentralState::PoweredOff) => return,
            _ => {}
        }
    }
    // An event subscription ending says nothing about the printer's connection state.
    future::pending::<()>().await;
}

pub async fn disconnect_monitor(adapter: Adapter, peripheral: Peripheral) -> DisconnectMonitor {
    // Subscribe immediately after connect, before service discovery and the model handshake.
    let events = timeout(STATUS_TIMEOUT, adapter.events())
        .await
        .ok()
        .and_then(Result::ok);
    async move {
        let id = peripheral.id();
        let listen = async {
            match events {
                Some(events) => wait_for_disconnect_event(events, &id).await,
                None => future::pending().await,
            }
        };
        let check = async {
            loop {
                match connection_state(&peripheral).await {
                    Some(false) => return,
                    None => {
                        // BlueZ can remove a peripheral before it converts its disconnect event.
                        if let Ok(Ok(devices)) =
                            timeout(STATUS_TIMEOUT, adapter.peripherals()).await
                        {
                            if devices.iter().all(|device| device.id() != id) {
                                return;
                            }
                        }
                    }
                    Some(true) => {}
                }
                sleep(Duration::from_secs(5)).await;
            }
        };
        futures::pin_mut!(listen, check);
        future::select(listen, check).await;
    }
    .boxed()
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use futures::stream;
    use uuid::Uuid;

    #[test]
    fn only_the_active_printer_or_adapter_power_off_ends_the_session() {
        let printer = PeripheralId::from(Uuid::new_v4());
        let other = PeripheralId::from(Uuid::new_v4());
        let unrelated = [
            CentralEvent::DeviceDisconnected(other),
            CentralEvent::DeviceConnected(printer.clone()),
            CentralEvent::StateUpdate(CentralState::Unknown),
            CentralEvent::StateUpdate(CentralState::PoweredOn),
        ];
        let ignores_unrelated = stream::iter(unrelated.clone()).boxed();
        assert!(wait_for_disconnect_event(ignores_unrelated, &printer)
            .now_or_never()
            .is_none());
        for event in [
            CentralEvent::DeviceDisconnected(printer.clone()),
            CentralEvent::StateUpdate(CentralState::PoweredOff),
        ] {
            let events = stream::iter(unrelated.clone().into_iter().chain([event])).boxed();
            assert!(wait_for_disconnect_event(events, &printer)
                .now_or_never()
                .is_some());
        }
    }
}

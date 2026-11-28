use kazzu_audio::EngineHandle;
use kazzu_core::PlaybackEvent;

use crate::controller::{Msg, MsgSink};

/// Lê os eventos do motor numa thread e os entrega à UI (agrupando posições, que chegam a ~10 Hz).
pub(crate) fn spawn_engine_bridge(engine: &EngineHandle, sink: MsgSink) -> std::io::Result<()> {
    let rx = engine.events().clone();
    std::thread::Builder::new()
        .name("kazzu-events".into())
        .spawn(move || {
            while let Ok(first) = rx.recv() {
                let mut batch = vec![first];
                batch.extend(rx.try_iter());
                // só a última posição do lote interessa
                let last_position = batch
                    .iter()
                    .rposition(|e| matches!(e, PlaybackEvent::Position { .. }));
                for (i, ev) in batch.into_iter().enumerate() {
                    if matches!(ev, PlaybackEvent::Position { .. }) && Some(i) != last_position {
                        continue;
                    }
                    sink(Msg::Engine(ev));
                }
            }
        })?;
    Ok(())
}

/// Lê os eventos de streaming (URL vencida, rede caiu) e os entrega à UI.
pub(crate) fn spawn_stream_bridge(engine: &EngineHandle, sink: MsgSink) -> std::io::Result<()> {
    let rx = engine.stream_events().clone();
    std::thread::Builder::new()
        .name("kazzu-stream-events".into())
        .spawn(move || {
            while let Ok(ev) = rx.recv() {
                sink(Msg::Stream(ev));
            }
        })?;
    Ok(())
}

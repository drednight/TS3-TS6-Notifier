//! Mock data source — demo roster with animated talking states.

use crate::model::{now_ms, OverlayState, UserState};
use crate::plugin::Emit;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

pub struct MockConnector {
    stop: Arc<AtomicBool>,
}

impl MockConnector {
    pub fn start(emit: Emit) -> MockConnector {
        let stop = Arc::new(AtomicBool::new(false));
        {
            let stop = stop.clone();
            thread::spawn(move || {
                let names = ["Алиса", "Борис", "Вика", "Гоша", "Даша"];
                let mut tick: u64 = 0;
                while !stop.load(Ordering::SeqCst) {
                    tick += 1;
                    let mut st = OverlayState::empty("mock", "live", "");
                    st.server_name = "Mock Server".into();
                    st.channel_name = "General".into();
                    st.channel_id = "5".into();
                    st.self_id = Some("1".into());
                    st.users = names
                        .iter()
                        .enumerate()
                        .map(|(i, name)| {
                            let mut u = UserState::new((i + 1).to_string(), *name);
                            u.channel_id = 5;
                            u.is_self = i == 0;
                            u.talking = (tick / 3 + i as u64) % 5 == 0;
                            u.input_muted = i == 4;
                            u
                        })
                        .collect();
                    st.updated_at = now_ms();
                    emit(st);
                    thread::sleep(Duration::from_millis(1200));
                }
            });
        }
        MockConnector { stop }
    }

    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

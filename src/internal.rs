use std::sync::Once;

static INIT: Once = Once::new();

#[allow(dead_code)] // this function is used in tests
pub(crate) fn init_logger() {
    INIT.call_once(|| {
        #[cfg(test)]
        env_logger::builder()
            .filter_level(log::LevelFilter::Debug)
            .init();
    });
}

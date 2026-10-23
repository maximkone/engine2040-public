//! The web page's pure side, black box: the backend probe's decision and the receipt line the
//! page prints before its first frame. The wasm shell in `crates/web/src/shell.rs` asks the
//! browser and then calls exactly these `engine_web::limits` functions.

mod backend_choice;

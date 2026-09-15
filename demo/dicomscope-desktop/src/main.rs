//! Desktop entry point. The app itself lives in the library so the iOS
//! bundle can call it from a C `main`.

#![forbid(unsafe_code)]

fn main() {
    dicomscope_desktop::run(std::env::args().skip(1).collect());
}

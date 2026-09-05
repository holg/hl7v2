//! wgpu rendering. Browser only, except the shader validation test below.

#[cfg(target_arch = "wasm32")]
pub mod gpu;

#[cfg(target_arch = "wasm32")]
pub use gpu::{Renderer, Uniforms};

/// The WGSL source, kept here so the host test and the renderer share it.
pub const SHADER: &str = include_str!("shader.wgsl");

#[cfg(test)]
mod tests {
    use super::SHADER;

    /// A WGSL error in the browser is a silent black canvas plus a console
    /// message; on the host it is this test failing.
    #[test]
    fn shader_parses_and_validates() {
        let module = naga::front::wgsl::parse_str(SHADER).unwrap_or_else(|e| {
            panic!("{}", e.emit_to_string(SHADER));
        });
        let mut validator = naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        );
        let info = validator
            .validate(&module)
            .unwrap_or_else(|e| panic!("{}", e.emit_to_string(SHADER)));
        let names: Vec<_> = module
            .entry_points
            .iter()
            .map(|e| e.name.as_str())
            .collect();
        assert_eq!(names, ["vs_main", "fs_main"]);
        let _ = info;
    }
}

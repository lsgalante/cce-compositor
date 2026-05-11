// River Wayland protocol bindings generated from XML

macro_rules! river_protocol {
    ($path:expr, [$($imports:path),*]) => {
        #[allow(dead_code, non_camel_case_types, unused_unsafe, unused_variables)]
        #[allow(non_upper_case_globals, non_snake_case, unused_imports, missing_docs, clippy::all)]
        pub mod generated {
            pub mod client {
                use wayland_client;
                use wayland_client::protocol::*;
                $(use $imports::{client::*};)*

                pub mod __interfaces {
                    use wayland_client::protocol::__interfaces::*;
                    $(use $imports::{client::__interfaces::*};)*
                    wayland_scanner::generate_interfaces!($path);
                }
                use self::__interfaces::*;

                wayland_scanner::generate_client_code!($path);
            }
        }
        pub use self::generated::client;
    };
}

pub mod river_window_management {
    river_protocol!("protocol/river-window-management-v1.xml", []);
}

pub mod river_xkb_bindings {
    river_protocol!("protocol/river-xkb-bindings-v1.xml",
        [crate::protocol::river_window_management::generated]);
}

pub mod river_layer_shell {
    river_protocol!("protocol/river-layer-shell-v1.xml",
        [crate::protocol::river_window_management::generated]);
}

pub mod river_input_management {
    river_protocol!("protocol/river-input-management-v1.xml",
        [crate::protocol::river_window_management::generated]);
}

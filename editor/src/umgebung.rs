//! Umgebung des Editors: TEST (Entwicklung, Zweig main) oder PROD (Release, Zweig prod, eigener Ordner).
//! Festgelegt beim Bauen: `OMSIGEN_UMGEBUNG=PROD` und `OMSIGEN_VERSION` setzt nur tools/release.py; ein normales
//! `cargo build` ist immer TEST. Beide bearbeiten dieselben Karten; getrennt sind Protokolle und Fenstertitel.

pub const NAME: &str = "OMSIgen";

/// "TEST" oder "PROD"
pub fn umgebung() -> &'static str {
    match option_env!("OMSIGEN_UMGEBUNG") {
        Some(u) if u.eq_ignore_ascii_case("PROD") => "PROD",
        _ => "TEST",
    }
}

pub fn prod() -> bool {
    umgebung() == "PROD"
}

/// Release-Version (Prod) bzw. Paketversion mit "-test"
pub fn version() -> String {
    match option_env!("OMSIGEN_VERSION") {
        Some(v) if prod() => v.to_string(),
        _ => format!("{}-test", env!("CARGO_PKG_VERSION")),
    }
}

/// Fenstertitel: "OMSIgen" (Prod) bzw. "OMSIgen TEST"
pub fn titel() -> String {
    if prod() { NAME.to_string() } else { format!("{NAME} TEST") }
}

#[cfg(test)]
mod tests {
    #[test]
    fn normaler_build_ist_test() {
        // ohne Release-Variablen gebaut: Test
        if option_env!("OMSIGEN_UMGEBUNG").is_none() {
            assert_eq!(super::umgebung(), "TEST");
            assert!(super::version().ends_with("-test"));
            assert_eq!(super::titel(), "OMSIgen TEST");
        }
    }
}

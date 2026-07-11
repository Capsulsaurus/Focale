//! Focale desktop application.

fn app_title() -> String {
    format!("Focale — pipeline v{}", focale_core::PIPELINE_VERSION)
}

fn main() {
    println!("{}", app_title());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_title_includes_pipeline_version() {
        assert_eq!(app_title(), "Focale — pipeline v1");
    }
}

use enzui_iced_core::localization::{CatalogBuilder, Locale, Localizer, preferred_locales};

pub(crate) fn localizer() -> Localizer {
    let mut catalog = CatalogBuilder::new();
    for (locale, resource) in [
        ("en-US", include_str!("../assets/locales/en-US.ftl")),
        ("de", include_str!("../assets/locales/de.ftl")),
        ("ar", include_str!("../assets/locales/ar.ftl")),
    ] {
        catalog
            .add_resource(locale, resource)
            .expect("embedded Reader catalog must be valid");
    }
    Localizer::new(catalog.build(), preferred_locales(), Locale::default())
}

use super::*;

#[test]
fn pdf_dates_validate_precision_ranges_and_timezones() {
    {
        let accepted = [
            ("D:2024", PdfDatePrecision::Year),
            ("D:202402", PdfDatePrecision::Month),
            ("D:20240229", PdfDatePrecision::Day),
            ("D:20000229", PdfDatePrecision::Day),
            ("D:2024022912", PdfDatePrecision::Hour),
            ("D:202402291230", PdfDatePrecision::Minute),
            ("D:20240229123059", PdfDatePrecision::Second),
            ("D:20240229123059Z", PdfDatePrecision::ZonedSecond),
            ("D:20240229123059+05'30'", PdfDatePrecision::ZonedSecond),
            ("D:20240229123059-11'45'", PdfDatePrecision::ZonedSecond),
        ];
        for (value, precision) in accepted {
            assert_eq!(validate_pdf_date(value), Ok(precision), "{value}");
        }
    }
    {
        for value in [
            "20240229",
            "D:0000",
            "D:202413",
            "D:20230229",
            "D:19000229",
            "D:20240431",
            "D:2024022924",
            "D:202402291260",
            "D:20240229123060",
            "D:202402Z",
            "D:20240229123059+24'00'",
            "D:20240229123059+05'60'",
            "D:20240229123059+0530",
            "D:20240229123059Q",
            "D:２０２４",
        ] {
            assert!(validate_pdf_date(value).is_err(), "accepted {value}");
        }
    }
}

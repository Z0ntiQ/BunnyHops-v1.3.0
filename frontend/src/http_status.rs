//! Кодовая таблица статус-кодов REST/HTTP API (реестр IANA, RFC 9110 и
//! связанные RFC). Единственный источник имени статуса в UI мока: название
//! правила и варианта ставится автоматически по выбранному коду
//! (правило пользователя 17.11.2026).

pub(crate) struct StatusClass {
    pub(crate) title: &'static str,
    pub(crate) codes: &'static [(u16, &'static str)],
}

/// Зарегистрированные статус-коды, сгруппированные по классам.
pub(crate) const STATUS_CLASSES: &[StatusClass] = &[
    StatusClass {
        title: "1xx — Informational",
        codes: &[
            (100, "Continue"),
            (101, "Switching Protocols"),
            (102, "Processing"),
            (103, "Early Hints"),
        ],
    },
    StatusClass {
        title: "2xx — Successful",
        codes: &[
            (200, "OK"),
            (201, "Created"),
            (202, "Accepted"),
            (203, "Non-Authoritative Information"),
            (204, "No Content"),
            (205, "Reset Content"),
            (206, "Partial Content"),
            (207, "Multi-Status"),
            (208, "Already Reported"),
            (226, "IM Used"),
        ],
    },
    StatusClass {
        title: "3xx — Redirection",
        codes: &[
            (300, "Multiple Choices"),
            (301, "Moved Permanently"),
            (302, "Found"),
            (303, "See Other"),
            (304, "Not Modified"),
            (305, "Use Proxy"),
            (307, "Temporary Redirect"),
            (308, "Permanent Redirect"),
        ],
    },
    StatusClass {
        title: "4xx — Client Error",
        codes: &[
            (400, "Bad Request"),
            (401, "Unauthorized"),
            (402, "Payment Required"),
            (403, "Forbidden"),
            (404, "Not Found"),
            (405, "Method Not Allowed"),
            (406, "Not Acceptable"),
            (407, "Proxy Authentication Required"),
            (408, "Request Timeout"),
            (409, "Conflict"),
            (410, "Gone"),
            (411, "Length Required"),
            (412, "Precondition Failed"),
            (413, "Content Too Large"),
            (414, "URI Too Long"),
            (415, "Unsupported Media Type"),
            (416, "Range Not Satisfiable"),
            (417, "Expectation Failed"),
            (418, "I'm a teapot"),
            (421, "Misdirected Request"),
            (422, "Unprocessable Content"),
            (423, "Locked"),
            (424, "Failed Dependency"),
            (425, "Too Early"),
            (426, "Upgrade Required"),
            (428, "Precondition Required"),
            (429, "Too Many Requests"),
            (431, "Request Header Fields Too Large"),
            (451, "Unavailable For Legal Reasons"),
        ],
    },
    StatusClass {
        title: "5xx — Server Error",
        codes: &[
            (500, "Internal Server Error"),
            (501, "Not Implemented"),
            (502, "Bad Gateway"),
            (503, "Service Unavailable"),
            (504, "Gateway Timeout"),
            (505, "HTTP Version Not Supported"),
            (506, "Variant Also Negotiates"),
            (507, "Insufficient Storage"),
            (508, "Loop Detected"),
            (510, "Not Extended"),
            (511, "Network Authentication Required"),
        ],
    },
];

/// Имя зарегистрированного статуса по кодовой таблице стандарта.
pub(crate) fn status_phrase(code: u16) -> Option<&'static str> {
    STATUS_CLASSES
        .iter()
        .flat_map(|c| c.codes.iter())
        .find(|(c, _)| *c == code)
        .map(|(_, p)| *p)
}

/// Автоматическое название по коду: «201 Created»; незарегистрированный код —
/// только цифры.
pub(crate) fn status_label(code: u64) -> String {
    match status_phrase(code as u16) {
        Some(p) if code <= u16::MAX as u64 => format!("{code} {p}"),
        _ => code.to_string(),
    }
}

/// Только название статуса, без номера: «OK» / «Not Found». Номер живёт в поле
/// Status — на экране он один раз (правило «экономика места», 18.11.2026).
pub(crate) fn status_name(code: u64) -> String {
    match status_phrase(code as u16) {
        Some(p) if code <= u16::MAX as u64 => p.to_string(),
        _ => code.to_string(),
    }
}

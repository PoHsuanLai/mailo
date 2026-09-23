//! Lucide geometry, one constant per glyph, transcribed exactly from the design's `ICON`
//! table. ISC licence; see `super`.

use super::Shape;

pub(super) const INBOX: &[Shape] = &[
    Shape::Path("M22 12h-6l-2 3h-4l-2-3H2"),
    Shape::Path(
        "M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z",
    ),
];

pub(super) const STAR: &[Shape] = &[Shape::Path(
    "M12 2.5l2.9 5.88 6.49.94-4.7 4.58 1.11 6.46L12 17.31l-5.8 3.05 1.1-6.46-4.69-4.58 6.48-.94z",
)];

pub(super) const ARCHIVE: &[Shape] = &[
    Shape::Rect {
        x: "2",
        y: "3",
        width: "20",
        height: "5",
        rx: "1",
    },
    Shape::Path("M4 8v11a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8"),
    Shape::Path("M10 12h4"),
];

pub(super) const CLOCK: &[Shape] = &[
    Shape::Circle {
        cx: "12",
        cy: "12",
        r: "10",
    },
    Shape::Path("M12 6v6l4 2"),
];

pub(super) const TRASH: &[Shape] = &[
    Shape::Path("M3 6h18"),
    Shape::Path("M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6"),
    Shape::Path("M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"),
    Shape::Path("M10 11v6"),
    Shape::Path("M14 11v6"),
];

pub(super) const MAIL: &[Shape] = &[
    Shape::Rect {
        x: "2",
        y: "4",
        width: "20",
        height: "16",
        rx: "2",
    },
    Shape::Path("m22 7-8.97 5.7a1.94 1.94 0 0 1-2.06 0L2 7"),
];

pub(super) const MAIL_OPEN: &[Shape] = &[
    Shape::Path(
        "M21.2 8.4c.5.38.8.97.8 1.6v10a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V10a2 2 0 0 1 .8-1.6l8-6a2 2 0 0 1 2.4 0z",
    ),
    Shape::Path("m22 10-8.97 5.7a1.94 1.94 0 0 1-2.06 0L2 10"),
];

pub(super) const TAG: &[Shape] = &[
    Shape::Path(
        "M12.586 2.586A2 2 0 0 0 11.172 2H4a2 2 0 0 0-2 2v7.172a2 2 0 0 0 .586 1.414l8.704 8.704a2.426 2.426 0 0 0 3.42 0l6.58-6.58a2.426 2.426 0 0 0 0-3.42z",
    ),
    // The design fills this dot (`fill="currentColor" stroke="none"`). Stroke and fill
    // are CSS `.ic`, not attributes, so only the circle geometry is stored.
    Shape::Circle {
        cx: "7.5",
        cy: "7.5",
        r: ".6",
    },
];

pub(super) const REFRESH: &[Shape] = &[
    Shape::Path("M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8"),
    Shape::Path("M21 3v5h-5"),
    Shape::Path("M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16"),
    Shape::Path("M8 16H3v5"),
];

pub(super) const SEND: &[Shape] = &[
    Shape::Path(
        "M14.54 21.69a.5.5 0 0 0 .93-.03l6.5-19a.5.5 0 0 0-.63-.63l-19 6.5a.5.5 0 0 0-.03.93l7.93 3.18a2 2 0 0 1 1.11 1.11z",
    ),
    Shape::Path("m21.85 2.15-10.94 10.94"),
];

pub(super) const COMMAND: &[Shape] = &[Shape::Path(
    "M15 6v12a3 3 0 1 0 3-3H6a3 3 0 1 0 3 3V6a3 3 0 1 0-3 3h12a3 3 0 1 0-3-3",
)];

pub(super) const COLUMNS: &[Shape] = &[
    Shape::Rect {
        x: "3",
        y: "3",
        width: "18",
        height: "18",
        rx: "2",
    },
    Shape::Path("M9 3v18"),
    Shape::Path("M15 3v18"),
];

pub(super) const GROUP: &[Shape] = &[
    Shape::Rect {
        x: "3",
        y: "3",
        width: "7",
        height: "7",
        rx: "1",
    },
    Shape::Rect {
        x: "3",
        y: "14",
        width: "7",
        height: "7",
        rx: "1",
    },
    Shape::Path("M14 4h7"),
    Shape::Path("M14 9h7"),
    Shape::Path("M14 15h7"),
    Shape::Path("M14 20h7"),
];

pub(super) const PANEL: &[Shape] = &[
    Shape::Rect {
        x: "3",
        y: "3",
        width: "18",
        height: "18",
        rx: "2",
    },
    Shape::Path("M15 3v18"),
];

pub(super) const SQUARE: &[Shape] = &[Shape::Rect {
    x: "5",
    y: "5",
    width: "14",
    height: "14",
    rx: "2",
}];

pub(super) const MAXIMIZE: &[Shape] = &[
    Shape::Path("M8 3H5a2 2 0 0 0-2 2v3"),
    Shape::Path("M21 8V5a2 2 0 0 0-2-2h-3"),
    Shape::Path("M3 16v3a2 2 0 0 0 2 2h3"),
    Shape::Path("M16 21h3a2 2 0 0 0 2-2v-3"),
];

pub(super) const CORNER: &[Shape] = &[
    Shape::Path("M20 4v7a4 4 0 0 1-4 4H4"),
    Shape::Path("m9 10-5 5 5 5"),
];

pub(super) const UNDO: &[Shape] = &[
    Shape::Path("M9 14 4 9l5-5"),
    Shape::Path("M4 9h10.5a5.5 5.5 0 0 1 0 11H11"),
];

pub(super) const CHECK: &[Shape] = &[Shape::Path("M20 6 9 17l-5-5")];

pub(super) const X_MARK: &[Shape] = &[Shape::Path("M18 6 6 18"), Shape::Path("m6 6 12 12")];

pub(super) const PAPERCLIP: &[Shape] = &[Shape::Path(
    "m16 6-8.41 8.59a2 2 0 0 0 2.82 2.82l8.42-8.58a4 4 0 0 0-5.66-5.66l-8.41 8.58a6 6 0 1 0 8.49 8.49L21 12",
)];

pub(super) const PEN: &[Shape] = &[Shape::Path(
    "M21.17 6.81a1 1 0 0 0-3.98-3.99L3.84 16.17a2 2 0 0 0-.5.83l-1.32 4.35a.5.5 0 0 0 .62.62l4.35-1.32a2 2 0 0 0 .83-.5z",
)];

pub(super) const KEY: &[Shape] = &[
    Shape::Path("m15.5 7.5 2.3 2.3a1 1 0 0 0 1.4 0l2.1-2.1a1 1 0 0 0 0-1.4L19 4"),
    Shape::Path("m21 2-9.6 9.6"),
    Shape::Circle {
        cx: "7.5",
        cy: "15.5",
        r: "5.5",
    },
];

pub(super) const FILE_PEN: &[Shape] = &[
    Shape::Path(
        "M12.659 22H18a2 2 0 0 0 2-2V8a2.4 2.4 0 0 0-.706-1.706l-3.588-3.588A2.4 2.4 0 0 0 14 2H6a2 2 0 0 0-2 2v9.34",
    ),
    Shape::Path("M14 2v5a1 1 0 0 0 1 1h5"),
    Shape::Path(
        "M10.378 12.622a1 1 0 0 1 3 3.003L8.36 20.637a2 2 0 0 1-.854.506l-2.867.837a.5.5 0 0 1-.62-.62l.836-2.869a2 2 0 0 1 .506-.853z",
    ),
];

pub(super) const OCTAGON_ALERT: &[Shape] = &[
    Shape::Path("M12 16h.01"),
    Shape::Path("M12 8v4"),
    Shape::Path(
        "M15.312 2a2 2 0 0 1 1.414.586l4.688 4.688A2 2 0 0 1 22 8.688v6.624a2 2 0 0 1-.586 1.414l-4.688 4.688a2 2 0 0 1-1.414.586H8.688a2 2 0 0 1-1.414-.586l-4.688-4.688A2 2 0 0 1 2 15.312V8.688a2 2 0 0 1 .586-1.414l4.688-4.688A2 2 0 0 1 8.688 2z",
    ),
];

pub(super) const PIN: &[Shape] = &[
    Shape::Path("M12 17v5"),
    Shape::Path(
        "M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7a1 1 0 0 1 1-1 2 2 0 0 0 0-4H8a2 2 0 0 0 0 4 1 1 0 0 1 1 1z",
    ),
];

pub(super) const REPLY: &[Shape] = &[
    Shape::Path("M20 18v-2a4 4 0 0 0-4-4H4"),
    Shape::Path("m9 17-5-5 5-5"),
];

pub(super) const REPLY_ALL: &[Shape] = &[
    Shape::Path("m12 17-5-5 5-5"),
    Shape::Path("M22 18v-2a4 4 0 0 0-4-4H7"),
    Shape::Path("m7 17-5-5 5-5"),
];

pub(super) const FORWARD: &[Shape] = &[
    Shape::Path("m15 17 5-5-5-5"),
    Shape::Path("M4 18v-2a4 4 0 0 1 4-4h12"),
];

pub(super) const SEARCH: &[Shape] = &[
    Shape::Path("m21 21-4.34-4.34"),
    Shape::Circle {
        cx: "11",
        cy: "11",
        r: "8",
    },
];

pub(super) const SETTINGS: &[Shape] = &[
    Shape::Path(
        "M9.671 4.136a2.34 2.34 0 0 1 4.659 0 2.34 2.34 0 0 0 3.319 1.915 2.34 2.34 0 0 1 2.33 4.033 2.34 2.34 0 0 0 0 3.831 2.34 2.34 0 0 1-2.33 4.033 2.34 2.34 0 0 0-3.319 1.915 2.34 2.34 0 0 1-4.659 0 2.34 2.34 0 0 0-3.32-1.915 2.34 2.34 0 0 1-2.33-4.033 2.34 2.34 0 0 0 0-3.831A2.34 2.34 0 0 1 6.35 6.051a2.34 2.34 0 0 0 3.319-1.915",
    ),
    Shape::Circle {
        cx: "12",
        cy: "12",
        r: "3",
    },
];

pub(super) const PANEL_LEFT: &[Shape] = &[
    Shape::Rect {
        x: "3",
        y: "3",
        width: "18",
        height: "18",
        rx: "2",
    },
    Shape::Path("M9 3v18"),
];

pub(super) const PLUS: &[Shape] = &[Shape::Path("M5 12h14"), Shape::Path("M12 5v14")];

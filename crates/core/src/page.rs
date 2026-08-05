use wasm_bindgen::JsValue;
use web_sys::Window;

/// Kind of a page of the site.
///
/// The site also puts a class on `<html>` (`list_all.css`, `mypage.css`, `item.css`,
/// `toppage.css`), but its JS adds that later, so at `document_start` it is absent. This
/// enum comes from the pathname.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageKind {
    /// Top page (tp_pc).
    Top,
    /// Work page (ci_pc).
    Work,
    /// Player (sc_d_pc).
    Player,
    /// A list (c_all_pc, mp_viw_pc, mpa_*).
    List,
    Other,
}

impl PageKind {
    pub fn detect(window: &Window) -> Result<Self, JsValue> {
        Ok(Self::from_path(&window.location().pathname()?))
    }

    pub fn from_path(path: &str) -> Self {
        let last = path.rsplit('/').next().unwrap_or_default();
        match last {
            "tp_pc" | "tp" => Self::Top,
            "ci_pc" => Self::Work,
            "sc_d_pc" => Self::Player,
            _ if last.starts_with("c_all_pc")
                || last.starts_with("mp_viw")
                || last.starts_with("mpa_") =>
            {
                Self::List
            }
            _ => Self::Other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::PageKind;

    #[test]
    fn detects_page_kinds_from_path() {
        assert_eq!(PageKind::from_path("/animestore/tp_pc"), PageKind::Top);
        assert_eq!(PageKind::from_path("/animestore/ci_pc"), PageKind::Work);
        assert_eq!(PageKind::from_path("/animestore/sc_d_pc"), PageKind::Player);
        assert_eq!(PageKind::from_path("/animestore/c_all_pc"), PageKind::List);
        assert_eq!(PageKind::from_path("/animestore/mp_viw_pc"), PageKind::List);
        assert_eq!(
            PageKind::from_path("/animestore/mpa_fav_pc"),
            PageKind::List
        );
        assert_eq!(PageKind::from_path("/animestore/mpa_cmp"), PageKind::List);
        assert_eq!(
            PageKind::from_path("/animestore/CF/search_index"),
            PageKind::Other
        );
    }
}

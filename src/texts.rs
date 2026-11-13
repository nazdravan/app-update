//! Dialog strings in Romanian, English and Russian.
//!
//! `{v}` stands for a version, `{p}` for a folder and `{e}` for an error text.

use crate::UpdateError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    Ro,
    Ru,
    En,
}

#[derive(Debug)]
pub struct Texts {
    pub check_button: &'static str,
    pub title: &'static str,
    pub checking: &'static str,
    pub up_to_date: &'static str,
    pub available: &'static str,
    pub update: &'static str,
    pub later: &'static str,
    pub link_only: &'static str,
    pub open_page: &'static str,
    pub downloading: &'static str,
    pub installing: &'static str,
    pub ready: &'static str,
    pub restart: &'static str,
    pub cancel: &'static str,
    pub close: &'static str,
    pub err_network: &'static str,
    pub err_rate: &'static str,
    pub err_not_found: &'static str,
    pub err_checksum: &'static str,
    pub err_denied: &'static str,
    pub err_not_writable: &'static str,
    pub err_other: &'static str,
}

static RO: Texts = Texts {
    check_button: "Caută actualizări",
    title: "Actualizare",
    checking: "Se caută versiunea nouă…",
    up_to_date: "Ai ultima versiune ({v}).",
    available: "Versiunea {v} e disponibilă.",
    update: "Actualizează",
    later: "Mai târziu",
    link_only: "Copia aceasta nu se poate actualiza singură. Descarc-o de pe pagina versiunii.",
    open_page: "Deschide pagina versiunii",
    downloading: "Se descarcă…",
    installing: "Se instalează…",
    ready: "Versiunea {v} e instalată. Repornește programul ca s-o folosești.",
    restart: "Repornește",
    cancel: "Anulează",
    close: "Închide",
    err_network: "Nu se poate ajunge la GitHub. Verifică internetul.",
    err_rate: "GitHub a primit prea multe cereri. Încearcă peste o oră.",
    err_not_found: "Nu s-a găsit nicio versiune publicată.",
    err_checksum: "Fișierul descărcat e deteriorat. Nu s-a schimbat nimic.",
    err_denied: "Instalarea a fost anulată sau nu s-a dat parola. Nu s-a schimbat nimic.",
    err_not_writable: "Dosarul {p} nu se poate scrie. Nu s-a schimbat nimic.",
    err_other: "Actualizarea nu a reușit: {e}",
};

static EN: Texts = Texts {
    check_button: "Check for updates",
    title: "Update",
    checking: "Checking for a new version…",
    up_to_date: "You have the latest version ({v}).",
    available: "Version {v} is available.",
    update: "Update",
    later: "Later",
    link_only: "This copy can't update itself. Download it from the release page.",
    open_page: "Open the release page",
    downloading: "Downloading…",
    installing: "Installing…",
    ready: "Version {v} is installed. Restart the program to use it.",
    restart: "Restart",
    cancel: "Cancel",
    close: "Close",
    err_network: "Can't reach GitHub. Check the internet connection.",
    err_rate: "GitHub got too many requests. Try again in an hour.",
    err_not_found: "No published version was found.",
    err_checksum: "The download is damaged. Nothing was changed.",
    err_denied: "The installation was cancelled or not authorised. Nothing was changed.",
    err_not_writable: "The folder {p} can't be written. Nothing was changed.",
    err_other: "The update failed: {e}",
};

static RU: Texts = Texts {
    check_button: "Проверить обновления",
    title: "Обновление",
    checking: "Поиск новой версии…",
    up_to_date: "У вас последняя версия ({v}).",
    available: "Доступна версия {v}.",
    update: "Обновить",
    later: "Позже",
    link_only: "Эта копия не обновляется сама. Скачайте её со страницы версии.",
    open_page: "Открыть страницу версии",
    downloading: "Загрузка…",
    installing: "Установка…",
    ready: "Версия {v} установлена. Перезапустите программу.",
    restart: "Перезапустить",
    cancel: "Отмена",
    close: "Закрыть",
    err_network: "Нет связи с GitHub. Проверьте интернет.",
    err_rate: "Слишком много запросов к GitHub. Попробуйте через час.",
    err_not_found: "Опубликованная версия не найдена.",
    err_checksum: "Загруженный файл повреждён. Ничего не изменено.",
    err_denied: "Установка отменена или не разрешена. Ничего не изменено.",
    err_not_writable: "Папка {p} недоступна для записи. Ничего не изменено.",
    err_other: "Обновление не удалось: {e}",
};

pub fn texts(l: Lang) -> &'static Texts {
    match l {
        Lang::Ro => &RO,
        Lang::Ru => &RU,
        Lang::En => &EN,
    }
}

impl Texts {
    /// The message for an error, or `None` for `Cancelled` (which shows nothing).
    pub fn error(&self, e: &UpdateError) -> Option<String> {
        Some(match e {
            UpdateError::Network(_) => self.err_network.to_string(),
            UpdateError::RateLimited => self.err_rate.to_string(),
            UpdateError::NotFound => self.err_not_found.to_string(),
            UpdateError::ChecksumMismatch => self.err_checksum.to_string(),
            UpdateError::Denied => self.err_denied.to_string(),
            UpdateError::NotWritable(p) => self
                .err_not_writable
                .replace("{p}", &p.display().to_string()),
            UpdateError::Cancelled => return None,
            UpdateError::BadResponse(_)
            | UpdateError::NoAsset
            | UpdateError::NoChecksum
            | UpdateError::Install(_) => self.err_other.replace("{e}", &e.to_string()),
        })
    }
}

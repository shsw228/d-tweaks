//! Watches the registered works for a new episode.
//!
//! # Why not the work page
//!
//! The episode list of a work is in the HTML of `ci_pc?workId=`, and that page is 108KB,
//! is rendered for the session (it sends 30 `Set-Cookie` headers) and has no `ETag`, no
//! `Last-Modified` and no `Cache-Control`, so a conditional request is not possible. One
//! check of one work would cost a full page render.
//!
//! `rest/WS030101?partId=` answers the same question in about 1KB (all measured):
//!
//! ```text
//! partDispNumber  "第20話"          the number, as the site writes it
//! partTitle       "一番じゃないやり方"
//! mainScenePath   the still of the episode, 288x162
//! nextPartId      "25767052"        the next episode, or ""
//! resultCd        "1" exists / "0" does not exist
//! ```
//!
//! `prevPartId` and `nextPartId` are the order of the site, and the chain is exactly the
//! episode list of the work page (measured: 25 links on the page, the same 25 ids in the
//! same order, and the chain ends). The ids are **not** in one sequence (a digest between
//! two episodes takes an id and is not in the chain), so the next id cannot be counted;
//! the interface must say it.
//!
//! So one check is: ask for the last episode that is known, read `nextPartId`. Nothing
//! new is one request. A new episode costs one more request per episode, and that request
//! also gives the number and the title, so no HTML is parsed anywhere.
//!
//! The interface also needs no account (measured without a cookie), so a check does not
//! depend on the session. A `fetch` of the service worker is cross-origin (the sender is
//! the extension), so the default `credentials: "same-origin"` sends no cookie, and this
//! module does not change that: the check must not carry the account of the user.
//!
//! # What keeps the traffic small
//!
//! | | |
//! |---|---|
//! | 1KB per check, not 108KB | `WS030101` instead of the work page |
//! | One request per work while nothing changes | the answer of the tail is the whole check |
//! | At most four requests at the same time | `MAX_CONCURRENT` is the throttle |
//! | `MAX_PER_RUN` works per run, oldest first | a long list spreads over runs |
//! | A work that does not change doubles its wait | `next_wait_minutes`, up to one day |
//! | A visit of the page is a free check | the content script sends the list it already has |
//! | No alarm while it cannot matter | off, or no work registered, means no alarm |
//! | Nothing while the browser is offline | `online` |
//! | A work that gives nothing for 30 days is removed | `STOP_AFTER_DAYS` |
//! | The still of the episode costs no request | `mainScenePath` comes with the answer |

use js_sys::Date;
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen_futures::JsFuture;
use web_sys::Response;

use d_tweaks_shared::watch as model;
use d_tweaks_shared::{chrome, json, settings};

use crate::{describe, log};

/// The name of the periodic alarm.
pub const ALARM: &str = "dt-episode-watch";

/// Episode information. Needs no account.
const PART_INFO_URL: &str = "https://animestore.docomo.ne.jp/animestore/rest/WS030101?partId=";
/// The same for an id that starts with `C`. The site makes the same choice
/// (`partid.indexOf("C") === 0` in its `common.js`).
const LIMITED_PART_INFO_URL: &str =
    "https://animestore.docomo.ne.jp/animestore/rest/WS040101?partId=";

/// Works to check in one run. A longer list is spread over the following runs, so the
/// traffic of one run has a limit that does not depend on how many works the user has.
const MAX_PER_RUN: usize = 20;
/// Requests that may be in flight at the same time.
///
/// An earlier version did one request at a time with a wait of one second between them.
/// That is 12 seconds for ten works, and the user waits for all of it when the button in
/// the popup is used, so it was too slow to be useful.
///
/// The cap is the throttle now, and the wait is gone. Four requests of about 1KB is far
/// less than the site itself sends for one page of its own (tens of requests for the
/// images and the scripts), and the interface has a shared cache of ten seconds
/// (`cache-control: s-maxage=10`, measured), so ten works now need under a second.
const MAX_CONCURRENT: usize = 4;
/// New episodes to follow in one check.
///
/// A work that was registered and then not checked for a long time can have some new
/// episodes; a chain without an end would be a defect of the interface, and this is the
/// limit against it. The rest arrives at the next run.
const MAX_CHAIN: usize = 30;
/// Doublings of the wait. 6 gives 64 times the interval, which the day limit cuts first.
const BACKOFF_STEPS: u32 = 6;
/// Days without a new episode after which a work is not watched any more.
///
/// The site has no flag that says "this work is finished": the COMPLETE mark of the site is
/// the **viewing** state of the user (`userInfo.memberFlags`, measured), and it also appears
/// on a work that is still running, so it cannot end a watch. The time since the last
/// episode is the signal that this extension already holds, and it costs no request.
///
/// A season break is about three months, so a work that comes back is registered again by
/// the user. 30 days is long enough that a weekly work is never stopped by mistake.
const STOP_AFTER_DAYS: f64 = 30.0;
/// Colour of the badge. Reads on the light and on the dark toolbar.
const BADGE_COLOUR: &str = "#c2410c";

const MINUTE_MS: f64 = 60.0 * 1000.0;
const DAY_MS: f64 = 24.0 * 60.0 * MINUTE_MS;

/// The fields of `WS030101` that this module uses.
struct PartInfo {
    /// `"1"` exists, `"0"` does not.
    exists: bool,
    work_title: Option<String>,
    number: Option<String>,
    title: Option<String>,
    /// The still of the episode. It comes with the answer, so the popup can show a picture
    /// without one more request.
    main_scene_path: Option<String>,
    /// The next episode of the chain. `None` at the end of the list.
    next_part_id: Option<String>,
}

fn global() -> Result<web_sys::WorkerGlobalScope, JsValue> {
    js_sys::global()
        .dyn_into::<web_sys::WorkerGlobalScope>()
        .map_err(|_| JsValue::from_str("not a worker global scope"))
}

/// Is the browser online?
///
/// A run while the network is down would only make failures, and a failure moves the
/// back-off, so an offline run would push the next real check away.
fn online() -> bool {
    js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("navigator"))
        .and_then(|navigator| js_sys::Reflect::get(&navigator, &JsValue::from_str("onLine")))
        .ok()
        .and_then(|value| value.as_bool())
        // Without the value, act as if online: a check that fails is not worse than no check
        .unwrap_or(true)
}

/// Read one episode. `None` when the request itself failed.
///
/// A reply that says "does not exist" is not a failure: it is the answer at the end of
/// the list, and the caller must be able to tell the two apart.
async fn part_info(part_id: &str) -> Option<PartInfo> {
    let base = if part_id.starts_with('C') {
        LIMITED_PART_INFO_URL
    } else {
        PART_INFO_URL
    };
    let url = format!("{base}{part_id}");
    let response: Response = JsFuture::from(global().ok()?.fetch_with_str(&url))
        .await
        .ok()?
        .dyn_into()
        .ok()?;
    if !response.ok() {
        log(&format!(
            "{part_id}: 話情報の取得に失敗 HTTP {}",
            response.status()
        ));
        return None;
    }
    let body = JsFuture::from(response.json().ok()?).await.ok()?;

    let text = |key: &str| json::get_string(&body, key).filter(|v| !v.trim().is_empty());
    Some(PartInfo {
        // The interface answers "1" for an episode that exists and "0" for an id that
        // has none. A number is also accepted: the field is a string in the measurement,
        // but a change to a number must not read as "does not exist".
        exists: text("resultCd").as_deref() == Some("1")
            || json::get_f64(&body, "resultCd") == Some(1.0),
        work_title: text("workTitle"),
        number: text("partDispNumber"),
        title: text("partTitle"),
        main_scene_path: text("mainScenePath"),
        next_part_id: text("nextPartId"),
    })
}

/// The wait until the next check of one work, in minutes.
///
/// `misses` are the checks in sequence that found nothing. Each one doubles the wait, so
/// a work that ended goes to the day limit and stays there, and a work that is running
/// goes back to the interval of the user the moment it gives an episode.
fn next_wait_minutes(base_minutes: f64, misses: u32) -> f64 {
    let factor = f64::from(2_u32.pow(misses.min(BACKOFF_STEPS)));
    (base_minutes * factor).min(settings::WATCH_INTERVAL_MAX)
}

/// Has this work given nothing for long enough to stop watching it?
///
/// `0` means "not known" (an entry of an older version), and such an entry is never
/// stopped: a work must not disappear because of a value that it never had.
fn is_stale(last_new_at: f64, now: f64) -> bool {
    last_new_at > 0.0 && now - last_new_at >= STOP_AFTER_DAYS * DAY_MS
}

// --- The storage. The service worker is the only writer (see `shared::watch`). ---

async fn load_works() -> Vec<model::Watched> {
    let keys = chrome::string_array(&[model::WORKS_KEY]);
    let Ok(stored) = chrome::local_get(&keys).await else {
        return Vec::new();
    };
    match json::get(stored.as_ref(), model::WORKS_KEY) {
        Some(value) => model::works_from_js(&value),
        None => Vec::new(),
    }
}

async fn save_works(works: &[model::Watched]) -> Result<(), JsValue> {
    let items = json::object(&[(model::WORKS_KEY, model::works_to_js(works)?.into())])?;
    chrome::local_set(&items).await
}

async fn load_news() -> Vec<model::Found> {
    let keys = chrome::string_array(&[model::NEWS_KEY]);
    let Ok(stored) = chrome::local_get(&keys).await else {
        return Vec::new();
    };
    match json::get(stored.as_ref(), model::NEWS_KEY) {
        Some(value) => model::news_from_js(&value),
        None => Vec::new(),
    }
}

async fn save_news(news: &[model::Found]) -> Result<(), JsValue> {
    let items = json::object(&[(model::NEWS_KEY, model::news_to_js(news)?.into())])?;
    chrome::local_set(&items).await
}

/// Put the number of unread entries on the toolbar icon.
pub async fn refresh_badge() {
    let count = model::unread(&load_news().await);
    let text = if count == 0 {
        String::new()
    } else {
        count.to_string()
    };
    if let Err(err) = chrome::set_badge(&text, BADGE_COLOUR).await {
        log(&format!("バッジを更新できません: {}", describe(&err)));
    }
}

/// Remove the badge. Used when the feature goes off: a number that nothing can open is
/// worse than no number.
async fn clear_badge() {
    if let Err(err) = chrome::set_badge("", BADGE_COLOUR).await {
        log(&format!("バッジを消せません: {}", describe(&err)));
    }
}

/// Put the entries at the front of the list, newest first, and cut the tail.
fn push_news(news: &mut Vec<model::Found>, found: Vec<model::Found>) {
    for entry in found.into_iter().rev() {
        news.insert(0, entry);
    }
    news.truncate(model::MAX_NEWS);
}

/// Is the watch on? Both the master switch and the feature must be on.
async fn feature_on() -> bool {
    settings::is_extension_enabled().await && settings::is_enabled(settings::EPISODE_WATCH).await
}

/// Make the alarm agree with the settings and with the list.
///
/// Called from `onInstalled`, `onStartup` and every change of a setting. Nothing runs
/// while the feature is off or no work is registered, so there is no alarm in that state.
pub async fn sync_alarm() {
    if !feature_on().await {
        chrome::clear_alarm(ALARM).await;
        clear_badge().await;
        return;
    }
    if load_works().await.is_empty() {
        chrome::clear_alarm(ALARM).await;
        return;
    }

    let period = settings::watch_interval_minutes().await;
    // `create` replaces the alarm and starts the period again, so a change of any other
    // setting would postpone the next check for ever. Only a different period writes.
    if chrome::alarm_period(ALARM).await == Some(period) {
        return;
    }
    // The first check does not run at the moment of the change: a user who tries the
    // switches would send one request per change.
    if let Err(err) = chrome::create_alarm(ALARM, period, 1.0) {
        log(&format!("新着の確認を予約できません: {}", describe(&err)));
        return;
    }
    log(&format!("新着の確認を {period} 分ごとに予約した"));
}

/// One run of the check. Returns the number of new episodes.
pub async fn run() -> usize {
    if !feature_on().await {
        chrome::clear_alarm(ALARM).await;
        clear_badge().await;
        log("新着の確認: 設定で無効なので何もしない");
        return 0;
    }
    if !online() {
        log("新着の確認: オフラインなので見送った");
        return 0;
    }

    let mut works = load_works().await;
    if works.is_empty() {
        chrome::clear_alarm(ALARM).await;
        log("新着の確認: 見張っている作品がない（作品ページの「新着を見張る」で登録します）");
        return 0;
    }

    let base = settings::watch_interval_minutes().await;
    let now = Date::now();

    // The works that are due, the one that waited longest first. A list that is longer
    // than `MAX_PER_RUN` is spread over the runs that follow, and the order makes sure
    // that no work is left out for ever.
    let mut due: Vec<usize> = (0..works.len())
        .filter(|index| works[*index].next_at <= now)
        .collect();
    due.sort_by(|a, b| {
        works[*a]
            .next_at
            .partial_cmp(&works[*b].next_at)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    if due.len() > MAX_PER_RUN {
        log(&format!(
            "確認するのは {} 件（残り {} 件は次の回に回す）",
            MAX_PER_RUN,
            due.len() - MAX_PER_RUN
        ));
        due.truncate(MAX_PER_RUN);
    }
    if due.is_empty() {
        // Every work is inside its wait. The next time comes from the back-off, so name it.
        let soonest = works
            .iter()
            .map(|work| work.next_at)
            .fold(f64::INFINITY, f64::min);
        log(&format!(
            "新着の確認: {} 件とも待ち時間の中（最短で約 {} 分後）",
            works.len(),
            ((soonest - now) / MINUTE_MS).max(0.0).round()
        ));
        return 0;
    }

    let checked = due.len();
    let mut found_all: Vec<model::Found> = Vec::new();
    let mut stop: Vec<String> = Vec::new();

    // `MAX_CONCURRENT` at a time. The batch is the throttle, and the state of the works is
    // written between two batches, never while requests are open.
    for chunk in due.chunks(MAX_CONCURRENT) {
        let jobs: Vec<Job> = chunk
            .iter()
            .map(|&index| Job {
                index,
                work_id: works[index].work_id.clone(),
                title: works[index].title.clone(),
                tail: works[index].tail.clone(),
            })
            .collect();

        for (index, outcome) in probe_batch(jobs).await {
            apply(&mut works[index], &outcome, base);

            // Stop watching a work that has given nothing for a long time.
            //
            // Only after an answer: a month without a network would otherwise end every
            // watch, and a failure is not "the work is finished".
            let work = &works[index];
            if outcome.answered
                && outcome.found.is_empty()
                && is_stale(work.last_new_at, Date::now())
            {
                log(&format!(
                    "{} {}: {STOP_AFTER_DAYS} 日増えていないので見張りを終了する",
                    work.work_id, work.title
                ));
                found_all.push(model::ended_row(&work.work_id, &work.title, Date::now()));
                stop.push(work.work_id.clone());
            }
            found_all.extend(outcome.found);
        }
    }

    if !stop.is_empty() {
        works.retain(|work| !stop.contains(&work.work_id));
    }
    if let Err(err) = save_works(&works).await {
        log(&format!("見張りの状態を保存できません: {}", describe(&err)));
    }
    if found_all.is_empty() {
        log(&format!("新着の確認: {checked} 件を見て、新着なし"));
        return 0;
    }

    // The rows that say "the watch ended" are in the same list, and they are already read,
    // so they are not in this number
    let count = found_all.iter().filter(|entry| !entry.ended).count();
    let mut news = load_news().await;
    push_news(&mut news, found_all);
    if let Err(err) = save_news(&news).await {
        log(&format!("新着を保存できません: {}", describe(&err)));
    }
    refresh_badge().await;
    if !stop.is_empty() {
        // The last work that goes removes the alarm
        sync_alarm().await;
    }
    if count > 0 {
        log(&format!("新着 {count} 件"));
    }
    count
}

/// What the site said about one work. No state of the work is in here (see `apply`).
struct Outcome {
    /// The new episodes, in the order of the site.
    found: Vec<model::Found>,
    /// Did the site answer about the episode that was asked for?
    ///
    /// `false` for a failed request, for a work that has no episode to ask about, and for
    /// an episode that the site does not have any more. The caller must not read any of
    /// those as "this work is finished".
    answered: bool,
    /// The last episode of the chain, when it moved.
    tail: Option<String>,
    /// The work title as the interface writes it, when it gave one.
    title: Option<String>,
}

impl Outcome {
    fn silent() -> Self {
        Self {
            found: Vec::new(),
            answered: false,
            tail: None,
            title: None,
        }
    }
}

/// What one check needs.
///
/// It owns its values, so the checks of one batch can run at the same time: a borrow of
/// the list of works would make them one after the other again.
struct Job {
    /// Where the work is in the list, so `apply` finds it again.
    index: usize,
    work_id: String,
    title: String,
    tail: Option<String>,
}

/// Ask the site about one work. Changes nothing.
async fn probe_work(job: &Job) -> Outcome {
    let Some(tail) = job.tail.clone() else {
        // Nothing to compare with, so a request would answer nothing. The work page sets
        // this the next time the user opens it (`observed`).
        log(&format!(
            "{}: 基準になる話がないので確認しない",
            job.work_id
        ));
        return Outcome::silent();
    };

    let Some(info) = part_info(&tail).await else {
        // A failed request is not "nothing new": the tail stays and the wait grows, so a
        // site that is down does not get one request per minute.
        return Outcome::silent();
    };

    if !info.exists {
        // The site removed the episode that was the tail. The chain cannot be followed
        // from it, and a visit of the work page repairs it.
        log(&format!(
            "{}: 基準の話 {tail} が無くなっている（作品ページを開くと直ります）",
            job.work_id
        ));
        return Outcome::silent();
    }

    // The title of the interface has no number of episodes in it, so it is the better one
    let title = info.work_title.clone().unwrap_or_else(|| job.title.clone());
    let mut found = Vec::new();
    let mut moved_tail = None;
    let mut next = info.next_part_id;

    // The chain must be followed one step at a time: each step needs the id of the one
    // before it. This runs only when an episode was added, which is rare.
    while let Some(part_id) = next {
        if found.len() >= MAX_CHAIN {
            log(&format!(
                "{}: 一度に辿るのは {MAX_CHAIN} 話まで（残りは次の回に）",
                job.work_id
            ));
            break;
        }
        // Only a new episode costs this request, and it also gives the number and the title
        let Some(episode) = part_info(&part_id).await else {
            break;
        };
        if !episode.exists {
            break;
        }
        found.push(model::Found {
            work_id: job.work_id.clone(),
            work_title: episode.work_title.clone().unwrap_or_else(|| title.clone()),
            part_id: part_id.clone(),
            number: episode.number,
            title: episode.title,
            thumb: episode.main_scene_path,
            at: Date::now(),
            seen: false,
            ended: false,
        });
        moved_tail = Some(part_id);
        next = episode.next_part_id;
    }

    Outcome {
        found,
        answered: true,
        tail: moved_tail,
        title: info.work_title,
    }
}

/// Run a batch of checks at the same time.
///
/// `Promise.all` over the tasks, because this crate has no async runtime of its own. WASM
/// is one thread and no borrow is held across an `await`, so a `RefCell` for the results
/// cannot be borrowed twice.
async fn probe_batch(jobs: Vec<Job>) -> Vec<(usize, Outcome)> {
    use std::cell::RefCell;
    use std::rc::Rc;

    /// One place per job in the batch, filled by the task that runs it.
    type Slots = Rc<RefCell<Vec<Option<(usize, Outcome)>>>>;

    let slots: Slots = Rc::new(RefCell::new((0..jobs.len()).map(|_| None).collect()));

    let promises = js_sys::Array::new();
    for (slot, job) in jobs.into_iter().enumerate() {
        let slots = Rc::clone(&slots);
        promises.push(&wasm_bindgen_futures::future_to_promise(async move {
            let outcome = probe_work(&job).await;
            slots.borrow_mut()[slot] = Some((job.index, outcome));
            Ok(JsValue::UNDEFINED)
        }));
    }

    // Every task ends with `Ok`, so this cannot reject
    let _ = JsFuture::from(js_sys::Promise::all(&promises)).await;
    slots.borrow_mut().drain(..).flatten().collect()
}

/// Put the answer of the site into the work.
///
/// Separate from `probe_work`, because that one runs at the same time as three others and
/// may not hold the list of works while it does.
fn apply(work: &mut model::Watched, outcome: &Outcome, base: f64) {
    let now = Date::now();
    if let Some(title) = &outcome.title {
        work.title = title.clone();
    }
    if let Some(tail) = &outcome.tail {
        work.tail = Some(tail.clone());
    }
    if outcome.answered {
        work.checked_at = now;
    }

    if outcome.found.is_empty() {
        work.misses = work.misses.saturating_add(1);
        if outcome.answered {
            log(&format!(
                "{} {}: 増えていない（{} 話目が最後、次は約 {} 分後）",
                work.work_id,
                work.title,
                work.tail.as_deref().unwrap_or("?"),
                next_wait_minutes(base, work.misses).round()
            ));
        }
    } else {
        // It is running, so go back to the interval of the user, and the clock of the
        // "nothing for a long time" rule starts again
        work.misses = 0;
        work.last_new_at = now;
        log(&format!(
            "{} {}: 新着 {} 件",
            work.work_id,
            work.title,
            outcome.found.len()
        ));
    }
    work.next_at = now + next_wait_minutes(base, work.misses) * MINUTE_MS;
}

// --- The messages of the work page and of the popup ---

/// One episode, as the work page reads it out of its own DOM.
struct Episode {
    part_id: String,
    number: Option<String>,
    title: Option<String>,
    thumb: Option<String>,
}

/// The episode list that came with a message.
fn episodes_of(message: &JsValue) -> Vec<Episode> {
    let Some(list) = json::get_array(message, "episodes") else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|entry| {
            let part_id = json::get_string(&entry, "partId")?;
            if part_id.trim().is_empty() {
                return None;
            }
            Some(Episode {
                part_id,
                number: json::get_string(&entry, "number").filter(|t| !t.trim().is_empty()),
                title: json::get_string(&entry, "title").filter(|t| !t.trim().is_empty()),
                thumb: json::get_string(&entry, "thumb").filter(|t| !t.trim().is_empty()),
            })
        })
        .collect()
}

/// The episodes after `tail`, in the order of the page.
///
/// The page holds the whole chain, so an episode that is behind the tail is new. An
/// unknown tail gives nothing: the whole list would then read as new.
fn after_tail(episodes: &[Episode], tail: Option<&str>) -> Vec<usize> {
    let Some(tail) = tail else { return Vec::new() };
    match episodes.iter().position(|e| e.part_id == tail) {
        Some(at) => (at + 1..episodes.len()).collect(),
        None => Vec::new(),
    }
}

/// A visit of the work page. It costs no request, so it is also a check.
///
/// The page has the list that a check would ask for, so this resyncs the tail (which
/// repairs a work whose tail the site removed) and records what was added.
///
/// The entries are **unread**, the same as the ones that the periodic check finds. An
/// earlier version marked them as read, because the user is looking at the episode list
/// and a badge would point at what is on the screen. That was wrong: a visit of a work page
/// is not proof that the user read the list (the page also opens for the summary, or from a
/// card of another list), and the badge is the only thing that says "something arrived". A
/// badge that is one click away from an answer is a smaller fault than an episode that is
/// never announced.
async fn observed(message: &JsValue, work: &mut model::Watched) -> Vec<model::Found> {
    let episodes = episodes_of(message);
    if episodes.is_empty() {
        // A page without the list (not signed in) says nothing about the work
        log(&format!(
            "{}: 作品ページにエピソード一覧がないので基準は変えない",
            work.work_id
        ));
        return Vec::new();
    }
    if let Some(title) = json::get_string(message, "title").filter(|t| !t.trim().is_empty()) {
        work.title = title;
    }

    let mut found = Vec::new();
    for index in after_tail(&episodes, work.tail.as_deref()) {
        let episode = &episodes[index];
        found.push(model::Found {
            work_id: work.work_id.clone(),
            work_title: work.title.clone(),
            part_id: episode.part_id.clone(),
            number: episode.number.clone(),
            title: episode.title.clone(),
            thumb: episode.thumb.clone(),
            at: Date::now(),
            seen: false,
            ended: false,
        });
    }

    // The tail is not in the list of the page, so the difference cannot be read.
    //
    // The tail still moves to the end of the list, because a tail that the site removed
    // would otherwise stop this work for ever. That step drops whatever came between, so it
    // must be visible in the log: without this line the work looks checked and silent, and
    // nothing says why (this is item 5 of known-issues.md).
    if found.is_empty()
        && let Some(tail) = work.tail.as_deref()
        && !episodes.iter().any(|episode| episode.part_id == tail)
    {
        log(&format!(
            "{}: 基準の話 {tail} が作品ページの一覧にない。一覧の最後（{}）を新しい基準にする（その間に増えた話は拾えない）",
            work.work_id,
            episodes.last().map(|e| e.part_id.as_str()).unwrap_or("?")
        ));
    }

    let now = Date::now();
    work.tail = episodes.last().map(|e| e.part_id.clone());
    work.checked_at = now;
    if !found.is_empty() {
        // Episodes were added, so the clock of the "nothing for a long time" rule starts again
        work.last_new_at = now;
    }
    // The page answered, so the next request can wait a full interval
    work.misses = 0;
    work.next_at = now + settings::watch_interval_minutes().await * MINUTE_MS;
    found
}

/// `{ ok: true, watched }`
fn state_reply(watched: bool) -> Result<JsValue, JsValue> {
    Ok(json::object(&[
        ("ok", JsValue::TRUE),
        ("watched", JsValue::from_bool(watched)),
    ])?
    .into())
}

/// Is this work watched? A visit of the page is also a free check.
pub async fn on_state(message: &JsValue) -> Result<JsValue, JsValue> {
    let Some(work_id) = json::get_string(message, "workId").filter(|id| !id.trim().is_empty())
    else {
        return Err(JsValue::from_str("作品 ID がありません"));
    };

    let mut works = load_works().await;
    let Some(index) = works.iter().position(|w| w.work_id == work_id) else {
        return state_reply(false);
    };

    let found = observed(message, &mut works[index]).await;
    if let Err(err) = save_works(&works).await {
        log(&format!("見張りの状態を保存できません: {}", describe(&err)));
    }
    if found.is_empty() {
        log(&format!(
            "{work_id}: 作品ページで確認（増えていない。基準 {}）",
            works[index].tail.as_deref().unwrap_or("なし")
        ));
        return state_reply(true);
    }

    log(&format!(
        "{work_id}: 作品ページで {} 件の追加を拾った（問い合わせなし）",
        found.len()
    ));
    let mut news = load_news().await;
    push_news(&mut news, found);
    if let Err(err) = save_news(&news).await {
        log(&format!("新着を保存できません: {}", describe(&err)));
    }
    // These entries are unread, so the icon must say so
    refresh_badge().await;
    state_reply(true)
}

/// Watch this work, with the episode list that the page already has as the start.
pub async fn on_add(message: &JsValue) -> Result<JsValue, JsValue> {
    let Some(work_id) = json::get_string(message, "workId").filter(|id| !id.trim().is_empty())
    else {
        return Err(JsValue::from_str("作品 ID がありません"));
    };

    let mut works = load_works().await;
    if works.iter().any(|w| w.work_id == work_id) {
        return state_reply(true);
    }
    if works.len() >= model::MAX_WORKS {
        return Ok(json::object(&[
            ("ok", JsValue::FALSE),
            ("full", JsValue::TRUE),
            ("max", JsValue::from_f64(model::MAX_WORKS as f64)),
        ])?
        .into());
    }

    let episodes = episodes_of(message);
    let now = Date::now();
    works.push(model::Watched {
        work_id: work_id.clone(),
        title: json::get_string(message, "title").unwrap_or_default(),
        // The page is the start, so the episodes that are there now are not new
        tail: episodes.last().map(|e| e.part_id.clone()),
        checked_at: now,
        next_at: now + settings::watch_interval_minutes().await * MINUTE_MS,
        misses: 0,
        // The clock of the "nothing for a long time" rule starts at the registration, so a
        // work that was over before it was registered also ends by itself
        last_new_at: now,
    });
    save_works(&works).await?;
    log(&format!(
        "{work_id}: 見張りに追加（基準 {} / 全 {} 件）",
        works
            .last()
            .and_then(|w| w.tail.clone())
            .unwrap_or_else(|| "なし".into()),
        works.len()
    ));
    // The first work makes the alarm
    sync_alarm().await;
    state_reply(true)
}

/// Stop watching this work. Its entries stay in the list.
pub async fn on_remove(message: &JsValue) -> Result<JsValue, JsValue> {
    let Some(work_id) = json::get_string(message, "workId").filter(|id| !id.trim().is_empty())
    else {
        return Err(JsValue::from_str("作品 ID がありません"));
    };

    let mut works = load_works().await;
    let before = works.len();
    works.retain(|w| w.work_id != work_id);
    if works.len() != before {
        save_works(&works).await?;
        log(&format!("{work_id}: 見張りをやめた"));
        // The last work removes the alarm
        sync_alarm().await;
    }
    state_reply(false)
}

/// The list, for the popup.
///
/// The watched works come with it: the popup can then name them, and a work can be dropped
/// without opening its page.
pub async fn on_list() -> Result<JsValue, JsValue> {
    let news = load_news().await;
    let works = load_works().await;
    Ok(json::object(&[
        ("ok", JsValue::TRUE),
        ("news", model::news_to_js(&news)?.into()),
        ("workList", model::works_to_js(&works)?.into()),
        ("enabled", JsValue::from_bool(feature_on().await)),
    ])?
    .into())
}

/// The list was on the screen, so the badge goes away.
pub async fn on_seen() -> Result<JsValue, JsValue> {
    let mut news = load_news().await;
    if model::unread(&news) > 0 {
        for entry in &mut news {
            entry.seen = true;
        }
        save_news(&news).await?;
    }
    clear_badge().await;
    Ok(json::object(&[("ok", JsValue::TRUE)])?.into())
}

/// Empty the list. The watched works stay.
pub async fn on_clear() -> Result<JsValue, JsValue> {
    save_news(&[]).await?;
    clear_badge().await;
    Ok(json::object(&[("ok", JsValue::TRUE)])?.into())
}

/// Check now, from the popup.
///
/// The back-off is for the runs of the alarm; a user who presses the button asks for an
/// answer now, so every work is due.
pub async fn on_check() -> Result<JsValue, JsValue> {
    let mut works = load_works().await;
    log(&format!(
        "新着の確認: ポップアップから手動で開始（登録 {} 件）",
        works.len()
    ));
    for work in &mut works {
        work.next_at = 0.0;
    }
    if let Err(err) = save_works(&works).await {
        log(&format!("見張りの状態を保存できません: {}", describe(&err)));
    }
    let count = run().await;
    Ok(json::object(&[
        ("ok", JsValue::TRUE),
        ("found", JsValue::from_f64(count as f64)),
    ])?
    .into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn episode(part_id: &str) -> Episode {
        Episode {
            part_id: part_id.into(),
            number: None,
            title: None,
            thumb: None,
        }
    }

    #[test]
    fn doubles_the_wait_and_stops_at_one_day() {
        // The interval of the user, until a check finds nothing
        assert_eq!(next_wait_minutes(60.0, 0), 60.0);
        assert_eq!(next_wait_minutes(60.0, 1), 120.0);
        assert_eq!(next_wait_minutes(60.0, 3), 480.0);
        // A work that ended settles at one request a day
        assert_eq!(next_wait_minutes(60.0, 5), 1440.0);
        assert_eq!(next_wait_minutes(60.0, 99), 1440.0);
        // The shortest interval also stops at the same limit
        assert_eq!(next_wait_minutes(30.0, 6), 1440.0);
    }

    #[test]
    fn stops_a_work_only_after_the_wait_and_only_with_a_known_time() {
        // A real epoch in milliseconds: a small number would make the times negative, and
        // a negative time is the "not known" case of the rule
        let now = 1_800_000_000_000.0;
        let day = 24.0 * 60.0 * 60.0 * 1000.0;
        assert!(!is_stale(now - 29.0 * day, now));
        assert!(is_stale(now - 30.0 * day, now));
        assert!(is_stale(now - 400.0 * day, now));
        // An entry of an older version has no time, and it must not disappear
        assert!(!is_stale(0.0, now));
    }

    #[test]
    fn takes_the_episodes_behind_the_tail() {
        let list = [
            episode("25767001"),
            episode("25767025"),
            episode("25767051"),
            episode("25767052"),
        ];
        assert_eq!(after_tail(&list, Some("25767025")), vec![2, 3]);
        // The tail is the last one: nothing was added
        assert_eq!(after_tail(&list, Some("25767052")), Vec::<usize>::new());
    }

    #[test]
    fn reports_nothing_without_a_known_tail() {
        let list = [episode("25767001"), episode("25767002")];
        // Without a tail the whole list would read as new, which is a false report
        assert_eq!(after_tail(&list, None), Vec::<usize>::new());
        // A tail that the page does not have (the site removed it) is the same case
        assert_eq!(after_tail(&list, Some("25767099")), Vec::<usize>::new());
    }

    #[test]
    fn keeps_the_newest_entry_first_and_cuts_the_tail() {
        let make = |part_id: &str| model::Found {
            work_id: "1".into(),
            work_title: "作品".into(),
            part_id: part_id.into(),
            number: None,
            title: None,
            thumb: None,
            at: 0.0,
            seen: false,
            ended: false,
        };
        let mut news = vec![make("old")];
        push_news(&mut news, vec![make("a"), make("b")]);
        // The list of one check keeps its order, and the newest check is in front
        let ids: Vec<&str> = news.iter().map(|e| e.part_id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "old"]);

        let mut full: Vec<model::Found> =
            (0..model::MAX_NEWS).map(|i| make(&i.to_string())).collect();
        push_news(&mut full, vec![make("newest")]);
        assert_eq!(full.len(), model::MAX_NEWS);
        assert_eq!(full[0].part_id, "newest");
    }
}

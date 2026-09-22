# План рефактора — 2026-09-22

Многосессионный план по итогам обхода `code_smell` + clippy (2026-09-22). Каждая фаза —
отдельный коммит (фаза 6 — серия коммитов), самостоятельно проверяемый и откатываемый.
Решения ниже приняты заранее: сессия, взявшая фазу, их **исполняет**, а не пересматривает.
Вопрос хозяину — только если код опровергает посылку фазы (тогда записать, что именно
опровергло, со ссылкой на `файл:строку`).

## ▶ С чего начать новую сессию

1. Прочитать этот файл целиком и раздел «Landmines» в
   [violin_trainer_plan.md](violin_trainer_plan.md) (там же сказано, почему `cargo clippy`
   печатает 4 предупреждения манифеста и это не код).
2. Взять **первую фазу без ✅** в таблице статуса. Порядок важен: см. «Зависимости».
3. После коммита фазы — поставить ✅ + тему коммита в таблицу (SHA не писать: история
   этого репо уже переписывалась, см. `memory/doc_shas_died_in_history_rewrite.md`).

## Статус

| Фаза | Суть | Статус |
|---|---|---|
| Ф1 | Отравленный мьютекс = паника, а не тихий дефолт | ✅ «refactor(audio): отравленный мьютекс = паника, а не тихий дефолт» |
| Ф2 | Один цикл аудио-воркера вместо двух копий | ✅ «refactor(audio): один цикл аудио-воркера вместо двух копий» |
| Ф3 | `core_types` не зависит от UI (`Scale` → `Mark`) | ✅ «refactor(ui): impl Mark for Scale переезжает к трейту — core_types без egui» |
| Ф4 | Распил `audio/native/imp.rs` по швам | ✅ «refactor(audio): распил native/imp.rs — drone, workers, capture, output» (⚠ `imp.rs` = 1038, не < 1000 — см. раздел фазы) |
| Ф5 | Мелочи: `total_cmp`, мёртвая `egui` в workspace | ✅ «chore: total_cmp в ранжировании гамм, мёртвая egui из workspace» |
| Ф6а | `Hz` / `Midi` + единственная конверсия | ⬜ |
| Ф6б | `SampleRate` | ⬜ |
| Ф6в | Целые MIDI-ноты и единый `BankRange` | ⬜ |
| Ф7 | Снять 22 `pub use` из `audio/mod.rs` | ⬜ |

**Зависимости.** Ф2 после Ф1 (гейт резонатора переписывается в Ф1). Ф4 после Ф2 (воркеры
переезжают один раз, уже слитыми). Ф6а → Ф6б → Ф6в строго по порядку. Ф7 последней:
она трогает импорты в 13 файлах, и до неё Ф6 успеет их поменять — делать это дважды
незачем. Ф3 и Ф5 независимы, их можно брать когда угодно.

## Общие правила для всех фаз

**Гейт каждой фазы** (все четыре, охват называть в отчёте):

```
cargo clippy --all-targets                                   # хост; 0 предупреждений кода
cargo check --target wasm32-unknown-unknown --lib --bins     # веб (НЕ --all-targets — падает всегда, см. memory/module_layout_and_target_gates.md)
cargo check --target aarch64-linux-android --lib             # Android
cargo test --release --lib --bins --tests                    # счётчик тестов сверять с базовым
```

Базовый счётчик тестов снять ДО первой правки фазы и записать в сообщение коммита
(«N passed до / N+k после»). На момент написания плана (2026-09-22, до Ф1):
**lib 185 passed / 15 ignored, `tests/pill_layout.rs` 2 passed**, бинарники 0 тестов.
После Ф1: **lib 186** (+1 тест отравления). ⚠ `release_ghosts_are_written_after_a_note`
флейкает (~1 из 5 прогонов, в том числе на коде до Ф1) — см. «Попутные находки»; одиночное
падение именно его не считать регрессией фазы, а перегнать. Тесты гонять в `--release`: в debug кадр анализа съедает
весь бюджет (см. «фриз при RT-SWIPE = debug-оверран» в памяти), и тайминговые тесты
начинают врать.

**Оракул переноса кода** (Ф3, Ф4): `diff -w -B` старого и нового места; всё сверх
обёрток/видимости/переотступа — незамеченная правка логики.

**Дельта детекторов**: до фазы сохранить нужный отчёт `code_smell` в scratchpad, после —
сравнить. Приёмка по дельте, а не по «стало меньше».

**Форматирование**: `rustfmt <изменённые файлы>`, без `lib.rs`/`mod.rs`, если в них
менялось только объявление модуля (иначе rustfmt пройдёт по детям).

---

## Ф1 — отравленный мьютекс = паника

**Проблема.** Из 49 вызовов `.lock()` вне `src/bin/` только 16 делают `.unwrap()`.
Остальные глотают `PoisonError` тремя формами:

- `lock().map(|g| g.clone()).unwrap_or_default()` — подставляют **дефолтные настройки**
  (`core.rs:312`, `core.rs:422`, `imp.rs:341`, `imp.rs:432`);
- `lock().ok().and_then(..)` / `.map(..).unwrap_or_else(..)` — возвращают «ничего»
  (`imp.rs:300`, `:307`, `:322`, `:329`, `:370`, `:415`, `:958`);
- `if let Ok(mut s) = lock() { … }` — **молча пропускают запись** (`core.rs:217`, `:370`,
  `:538`, `:554`, `:662`, `imp.rs:345`, `:439`, `:464`, `:654`, `:665`, `:908`,
  `worker.rs:110`, `:139`, `match` в `worker.rs:158`);
- гейт резонатора `imp.rs:1132` (`unwrap_or(false)`) и камертон тест-ноты `imp.rs:1429`
  (`unwrap_or(440.0)`).

Мьютекс отравлен ⇒ другой поток уже упал с паникой посреди записи. Глотание прячет
первую панику: приложение продолжает жить с дефолтными настройками и пустым роллом,
ошибки нигде нет. Это ровно тот класс «нот нет, ошибки нет», который уже стоил сессии
(`memory/display_settings_must_not_steer_the_detector.md`, пятый случай).

**Решение.**
- Все формы → `.lock().unwrap()`. Без `expect`: `PoisonError` в сообщении паники и так
  называет причину, а стиль проекта — `unwrap()`.
- `parking_lot` НЕ вводить: он снимает отравление целиком, то есть узаконивает то же
  глотание.
- **Мина области видимости guard'а.** `if let Ok(mut s) = m.lock() { … }` держит guard до
  конца блока. Механическая замена на `let mut s = m.lock().unwrap();` продлевает его до
  конца *охватывающей* функции — это удержание замка на время последующей работы и
  возможный дедлок с повторным `lock()` ниже. Замена сохраняет блок:
  `{ let mut s = m.lock().unwrap(); … }` — или, если после блока в функции ничего нет,
  блок можно снять. Каждую замену читать глазами на предмет второго `lock()` того же
  мьютекса ниже по функции.
- `src/bin/mic_probe.rs:335, :359` — туда же (зонд, но правило одно).
- Комментарий у гейта резонатора (`imp.rs:1123-1127`) противоречит коду («если замок
  отравлен — молотим», а `unwrap_or(false)` паркует). После правки отравления нет вовсе —
  комментарий переписать в одно предложение про дедлайн.

**Тест, который падает без поведения.** В `core.rs` (`mod tests`): отравить
`Arc<Mutex<AnalysisSettings>>` (поток, паникующий с guard'ом в руке), вызвать
`AnalysisPipeline::push_samples` — `#[should_panic(expected = "PoisonError")]`. Вернуть
`unwrap_or_default()` ⇒ тест краснеет.

**DoD.** `rg -n '\.lock\(\)' src | rg -v 'lock\(\)\.unwrap\(\)'` — только многострочные
цепочки, где `.unwrap()` на следующей строке (их перечислить в коммите поимённо); ни одного
`unwrap_or*`/`.ok()`/`if let Ok` на результате `lock()`. `code_smell option-crutch`:
сайты `core.rs:312`, `core.rs:422`, `imp.rs:300/322/341/370/432/958/1132/1429` ушли из отчёта.

## Ф2 — один цикл аудио-воркера

**Проблема.** `start_analysis_worker` и `start_resonator_worker`
(`audio/native/imp.rs:1072-1164`) — одна и та же петля «вычерпать ≤4096 из кольца →
заснуть, если пусто → `push_samples`». Отличие одно: резонатор перед этим смотрит
дедлайн `resonator_wanted` и, если он в прошлом, **всё равно вычерпывает кольцо**, но не
считает (иначе кольцо переполнится и при пробуждении выльется пачкой старого звука).

**Решение.**
- Одна функция `start_worker(sample_rate, cons, pipeline: WorkerPipeline, shared,
  settings, input_gain, input_level) -> AnalysisWorker`.
- ```rust
  enum WorkerPipeline {
      Analysis(AnalysisPipeline),
      /// Считает, только пока UI сдвигает дедлайн вперёд; иначе кольцо дренируется вхолостую.
      Resonator { pipeline: ResonatorPipeline, wanted: Arc<Mutex<Instant>> },
  }
  ```
  `enum`, а не трейт с двумя impl'ами: пайплайнов ровно два, и развилка — одна строка.
- Порядок в петле сохранить побайтно: `clear` → вычерпать → (резонатор) проверка
  дедлайна ⇒ `RESONATOR_PARK_SLEEP` → пусто ⇒ `ANALYSIS_IDLE_SLEEP` → `push_samples`.
  Проверку дедлайна делать ПОСЛЕ вычерпывания, как сейчас (иначе запаркованный воркер
  перестанет дренировать).
- Методы `AudioContext::start_analysis_worker`/`start_resonator_worker` (`imp.rs:875`,
  `:886`) остаются двумя тонкими обёртками — они собирают разные наборы `Arc` из контекста.
- Вычерпывание вынести в `fn pop_batch(cons: &mut SampleConsumer, batch: &mut Vec<f32>)`
  — его, кроме этих двух петель, никто не зовёт, но так петля читается за один экран.

**Оракул.** Существующий `a_take_replayed_through_the_engine_draws_a_line`
(`imp.rs:1766`) гоняет оба воркера через публичный API движка — он обязан остаться
зелёным. Отдельного теста на петлю не писать: поведение не меняется, а поток с `sleep`
в тесте — источник флейков.

**DoD.** Одна петля на `thread::spawn` в файле; `rg -c 'thread::spawn' imp.rs` уменьшился на 1.

## Ф3 — `core_types` не зависит от UI

**Проблема.** `core_types/scale.rs:1,9,19-37` тянет `egui::Color32`, реализует трейт
`Mark` из `ui::fretboard::draw` и держит палитру ступеней. Это единственный
межподсистемный цикл модулей (`code_smell dep-cycle`: `core_types::scale ⇄
ui::fretboard::draw`), и единственное место, где `core_types` знает про egui.

**Решение.** `impl Mark for &Scale` и `mark_some_scale` переезжают в
`ui/fretboard/draw.rs`, под трейт `Mark` (единственная реализация — рядом с трейтом;
`draw.rs` уже импортирует `Scale`). Закомментированные строки внутри
`mark_some_scale` (два `let scale = …`) — удалить при переезде. В `scale.rs` снять
`use eframe::egui::Color32` и `use crate::ui::fretboard::draw::Mark`.

**DoD.** `rg -n 'egui|crate::ui' src/core_types` → 0 строк. `code_smell dep-cycle`: cross-cut
0 (local `core_types::note ⇄ pitch` остаётся — это сиблинги одного домена, не трогаем).
`diff -w` переехавшего блока = только путь импорта.

## Ф4 — распил `audio/native/imp.rs`

**Проблема.** 1828 строк, из них 1742 код — самый большой код-файл репо. Подмодули
`devices`/`recorder`/`replay` уже вынесены; в теле остались ещё четыре независимых роли.

**Решение.** Новые файлы рядом с существующими `imp/devices.rs` и т. п.:

| Файл | Что переезжает (строки на 2026-09-22) |
|---|---|
| `imp/drone.rs` | `DroneSynth` + `impl`, `timbre_voice` (`1496-1743`) |
| `imp/workers.rs` | `AnalysisWorker`, слитый в Ф2 `start_worker` + `WorkerPipeline` + `pop_batch`, `analysis_ring` |
| `imp/capture.rs` | `PulseInputCapture`, `ActiveInput`, `ActiveCapture`, `InputFanout`, `build_input`, `build_pulse_input`, `pulse_i16_to_f32` |
| `imp/output.rs` | `build_monitor_output`, `play_test_note_thread`, `test_tone_samples` |

В `imp.rs` остаются `AudioEngine`, `Command`, `audio_thread_main`, `AudioContext`,
`audio_alog`/`report_stream_error` и `mod tests` (он end-to-end по движку).

- Видимость: `pub(super)` на каждый элемент и **поле**, которое читает `imp.rs` или сосед.
  Механика и ловушки — `memory/module_layout_and_target_gates.md` («главная работа
  переноса = видимость»).
- Юнит-тесты, если у переезжающих элементов они есть внутри `mod tests` `imp.rs`, едут
  вместе с предметом; `use super::*` в новом `mod tests` заменить поимённым импортом.
- Константы, которые использует только переехавший код, едут с ним; общие — остаются
  в `imp.rs` с `pub(super)`.
- `imp` не переименовывать (имя вписано в пути и комментарии `audio/types.rs`).

**DoD.** `wc -l imp.rs` < 1000. `diff -w -B` каждого переехавшего блока против
`git show HEAD:src/audio/native/imp.rs` = только видимость/импорты. Android-гейт
обязателен: `audio_alog` под `cfg(target_os = "android")`.

**Итог (2026-09-22): посылка DoD `< 1000` не подтвердилась — 1038 строк.** Таблица
перенесена целиком (drone 277, workers 155, capture 287, output 195 строк). Остаток
`imp.rs` — ровно то, что фаза велела оставить: `AudioEngine` + `impl` (`imp.rs:170-505`),
`audio_thread_main` (`:507-605`), `AudioContext` + `impl` (`:607-951`), `mod tests`
(`:953-1038`, 85 строк; кода без тестов — 952). Пятый распил сверх таблицы не
придумывался. Если порог важен, естественный кандидат — методы `AudioContext`, собирающие
пути захвата (`build_capture`/`build_cpal_capture`/`build_pulse_capture`/`build_replay_capture`,
`imp.rs:713-951`) → в
`capture.rs`; это решение хозяина, не механика фазы.
Механика: `item_mv` из `bur/rust_app/tools/mod_mv` + ручная доводка (он копирует `use`
источника относительными путями и теряет свободные `//`-комментарии над элементом:
осиротели баннеры DroneSynth/ActiveCapture/InputFanout и `// ±0.4 % высоты` — все
возвращены к своим элементам). Оракул — `git diff -w --color-moved=plain
--color-moved-ws=ignore-all-space`: вне перемещённых блоков только видимость, импорты,
doc-шапки модулей, путь одной doc-ссылки и два снятых баннера раздела.

## Ф5 — мелочи

- `app/scale_finder/ranking.rs:163-167`: `sort_by(|a, b| b.blended.total_cmp(&a.blended))`.
  NaN в `blended` не ожидается; `total_cmp` хотя бы делает порядок детерминированным,
  вместо «NaN равен всем» (что ломает транзитивность сортировки).
- `Cargo.toml:196`: удалить `egui = "0.36.2"` из `[workspace.dependencies]` — никто его не
  наследует (`eframe` реэкспортит egui). Снимает одно из 4 предупреждений манифеста;
  строку в «Landmines» `violin_trainer_plan.md` поправить на «3 предупреждения».
- Переименование бинарников в kebab-case — **НЕ в этом плане**: `index.html:17`
  адресует `data-bin="dsp_worker"`, а `wasm.rs:80` — `dsp_worker_loader.js`; это
  отдельное решение с проверкой trunk-сборки.
- `ui/fretboard/geometry.rs:73-116`: три свободные функции принимают одни и те же
  `&Range<f32>` + `&Range<Fret>` из полей `Fretboard` — **не трогать**: они приватные,
  зовутся из двух методов `Fretboard`, и структура-параметр здесь ничего не
  колокализует. Вывод `code_smell arg-clump` по ним закрыт как FP.

## Ф6 — единицы в типах

Правило репо: единица измерения — часть типа. Сейчас один и тот же домен живёт в
нескольких примитивах (`code_smell prim`):

- `sample_rate: f32` ×23 и `sample_rate: u32` ×16;
- MIDI: дробный `f32` (высота), целый `i32` (стафф, пианоролл), `usize` (хрома,
  подписи банка), `u8` внутри `PNote`;
- две разные структуры `BankRange`: `app/staff_panel.rs:597` (`i32`) и
  `app/take_roll.rs:229` (`f32`);
- формула midi↔Hz скопирована в рабочем коде 7 раз (`app.rs:614`, `:618`,
  `dsp/pyin.rs:172`, `:176`, `dsp/pitch_bench.rs:279`, `core.rs:576`) плюс копии в тестах
  (`rtswipe.rs:382`, `swipe.rs:611`, `:738`, `segmenter.rs:237`, `:269`), при том что
  крейт `resonators` уже экспортирует `midi_to_hz(midi, tuning)` и им пользуются
  `rtswipe.rs`, `resonator.rs`, `imp.rs`.

**Общие решения для Ф6.**
- Newtype'ы — `pub struct X(pub f32)` c `#[derive(Clone, Copy, Debug, PartialEq,
  PartialOrd)]`; без `Deref`, внутреннее значение раскрывается `.0` на границе с DSP-ядром
  (FFT, фильтры, `resonators`), UI-рисованием и сериализацией.
- Поля в `serde`-структурах (`audio/types.rs`, `worker_proto.rs`, персист настроек) —
  тип получает `#[serde(transparent)]`, чтобы формат на диске и в worker-протоколе не
  изменился. **Тест совместимости**: десериализовать литерал старого формата
  `AnalysisSettings` (снять из текущего `serde_json::to_string(&AnalysisSettings::default())`
  ДО правки и вписать в тест строкой).
- Массовая механика (сотни сайтов в Ф6б) — субагентом дешёвой модели по готовому
  рецепту; приёмка — гейтом фазы и `code_smell prim` дельтой, а НЕ отчётом агента.
- Вне Ф6 (записано, не делаем сейчас): `Cents`, `Seconds`, `view_lo/view_hi` роллов,
  `gamma`. Сначала посмотреть, как приживутся `Hz`/`Midi`.

### Ф6а — `Hz`, `Midi`, одна конверсия

- Дом: `core_types/pitch.rs`, рядом с `PNote` (целая нота) и `Interval`.
  - `pub struct Hz(pub f32)`, `pub struct Midi(pub f32)` — **дробная** высота
    (69.0 = A4, 69.5 = A4 + 50¢).
  - `impl Midi { pub fn to_hz(self, a4: Hz) -> Hz }` — через `resonators::midi_to_hz`
    (переиспользуем существующую, не пишем третью).
  - `impl Hz { pub fn to_midi(self, a4: Hz) -> Midi }` — `69 + 12·log2(f/a4)`;
    единственная копия формулы в репо, с комментарием.
  - `impl Hz { pub const A4_STANDARD: Hz = Hz(440.0); }`.
  - `impl From<PNote> for Midi`.
- Заменить все 7 рабочих копий и тестовые копии.
- **`pyin.rs:172-177` с жёстким 440 — не баг, но проверить и записать.** Там MIDI —
  внутренняя координата сетки треллиса: `freq → midi(440) → бин → midi → freq(440)`, и
  опорная частота в круговом пути сокращается. Перевести на `Hz::A4_STANDARD` и написать
  комментарий, почему здесь не камертон из настроек. Если при чтении окажется, что
  midi из pYIN уходит НАРУЖУ без обратной конверсии — это баг, чинить с тестом на
  камертоне 442.
- `concert_pitch_hz` / `reference_hz` в структурах и сигнатурах → `Hz` (31 упоминание
  `concert_pitch_hz`).
- Высота в истории мелодии (`MelodyFrame`, `pianoroll::PitchPoint::midi_f` и т. п.) →
  `Midi`. Прошлые поля `midi_f` переименовать в `midi` одновременно со сменой типа.

**DoD.** `rg -n '69\.0 \+ 12\.0|\(midi - 69\.0\) / 12\.0' src` → только `Hz::to_midi`.
`rg 'fn (freq_to_midi|midi_to_freq|hz_to_midi|midi_to_frequency|frequency_to_midi)' src` → 0.
Юнит-тест `Hz::to_midi ∘ Midi::to_hz = id` на 442 Гц (не на 440 — там путаница опоры не видна).

### Ф6б — `SampleRate`

- `pub struct SampleRate(pub u32)` в новом `src/audio/sample_rate.rs` (DSP знает только
  аудио-домен; `core_types` не нужен). Методы: `fn hz(self) -> f32` (для DSP-формул),
  `fn samples_in(self, d: Duration) -> usize`, `fn duration_of(self, n: usize) -> Duration`
  — только если на сайтах реально встречаются эти формулы (проверить `rg 'sample_rate as
  usize|/ sample_rate'` до того, как заводить).
- Базовое представление — `u32`: его отдаёт cpal и пишет WAV. Web `AudioContext.sampleRate`
  приходит `f32` → на границе `wasm.rs` конструктор с `assert!(sr.fract() == 0.0)`.
- Тестовые генераторы с `48_000.0f32` → `SampleRate(48_000)`.

**DoD.** `code_smell prim`: пары `sample_rate: f32` / `sample_rate: u32` исчезли из
NEWTYPE CANDIDATES; остаток (если есть) перечислен в коммите с причиной.

### Ф6в — целые MIDI-ноты и единый `BankRange`

- Целая нота везде — `PNote` (он уже есть и проверяет диапазон в `PNote::new`):
  `ui/staff.rs:284`, `:297`, `:418`, `staff_panel.rs:420`, `segmenter.rs:64`,
  `pianoroll.rs` `res_min_midi/res_max_midi`, `analysis_math.rs:167`,
  `scale_detect/chroma.rs:32-57`, `method_root.rs:27`, `resonator.rs:60`.
  Где код делает арифметику со знаком (`midi - 12`) — через `Interval`, как `PNote::add`.
- Один `BankRange { lo: PNote, hi: PNote }` в `audio/dsp/resonator.rs` (дом банка),
  методы `semitones()` и `span_f32()` для рисовальщиков. Обе локальные `BankRange`
  (`staff_panel.rs:597`, `take_roll.rs:229`) удалить.
  Инвариант `lo <= hi` — в конструкторе `BankRange::new`, `assert!`; комментарий
  `staff_panel.rs:603-604` про «вырожденный диапазон» переписать: вырожденного больше
  нет, `semitones()` ≥ 0.
- `ResonatorViewSettings.min_midi/max_midi` → `BankRange` (с `serde(transparent)` на
  `PNote`, если его ещё нет, — проверить формат тестом совместимости из общих правил).

**DoD.** `rg -n 'min_midi: (usize|i32)|max_midi: (usize|i32)' src` → 0.
`rg -n 'struct BankRange' src` → 1.

## Ф7 — снять реэкспорты `audio/mod.rs`

**Проблема.** `src/audio/mod.rs:28-50` реэкспортирует 22 типа из приватного `mod types`
вопреки запрету на `pub use`; импортёров — 13 файлов (`rg -l 'crate::audio::(\{|[A-Z])'
src`; в памяти от 09-03 стояло 27 — пересчитать на старте).

**Решение.**
- `mod types` → `pub mod types`; импорты во всех 13 файлах → `crate::audio::types::X`.
- `pub use worker::worker_entry` → `pub mod worker` (под тем же `cfg(wasm32)`), вызов в
  `src/bin/dsp_worker.rs:9` → `fretboard::audio::worker::worker_entry()`.
- `pub use native::imp::AudioEngine` / `pub use wasm::AudioEngine` **остаются**: это
  cfg-развилка одного имени на две платформы, а не сокрытие происхождения. Над ними
  одна строка комментария именно об этом.
- Хвост в `memory/module_layout_and_target_gates.md` («audio/mod.rs реэкспортит 22 типа»)
  пометить закрытым.

**DoD.** `rg -n 'pub use' src/audio/mod.rs` → ровно две строки `AudioEngine`. Wasm-гейт
обязателен (там `worker_entry`).

---

## Попутные находки (вне фаз, записаны с местом)

- **Флейк `audio::core::tests::release_ghosts_are_written_after_a_note`** (найден на
  базовом прогоне Ф1, 2026-09-22). Паника `core.rs:1199` — `assert!(!ghosts.is_empty())`:
  тест закрепляет известный баг «призраков после ноты», а число призраков зависит от
  стенного времени — `Rig::feed` держит каденс пайплайнов через `thread::sleep`, и под
  параллельной нагрузкой тест-раннера сон переспит, отчего гейт тишины и банк
  расходятся иначе. 2 падения на 6 прогонов `cargo test --release --lib`, одно из них — на
  коде ДО правок Ф1. Лечение — не ослаблять assert (тест это прямо запрещает), а убрать
  стенное время из `Rig` (часы пайплайна вместо `Instant::elapsed`) или починить сам баг.
- **`ActiveCapture.selected_id: String` с `""` вместо «устройство не выбрано»**
  (`audio/native/imp.rs:946`, `build_replay_capture`: `….clone().unwrap_or_default()`).
  Ф1 сняла с этого места глотание отравления, но сам фолбек `Option<String>` → пустая
  строка остался в отчёте `code_smell option-crutch`. Это доменный вопрос (что значит
  реплей без выбранного входа и кто читает `selected_id`), а не механика замка.
- **`try_lock` в колбэке дрона** (`imp.rs:704`, `build_drone_stream`) глотал и `WouldBlock`, и
  `Poisoned` одним `if let Ok`. Ф1 развела их: `WouldBlock` — держим прошлый снимок, как
  задумано; `Poisoned` — паника, как у всех `lock()`.

## Отброшено при обходе (не находки)

- `code_smell scan-loop`: `batch.drain(..)` в петле воркера — вычерпывание целиком, не
  вырезание из середины; `find` в петле `take_panel.rs:177` и `pyin.rs:124` — по
  коротким спискам (кандидаты/строки корпуса).
- `code_smell arg-clump`: четыре функции вкладок с `egui_tiles::{TabState, TileId, Tiles}`
  — сигнатуры трейта фреймворка.
- `code_smell nan-clamp`: `s.max(0.0).sqrt()` в `rtswipe.rs:499`, `swipe.rs:552` и
  `e.max(EMIT_EPS).ln()` в `trellis.rs:309` — зажим числового шума перед `sqrt`/`ln`,
  сознательный.
- `code_smell god-file`: `app/take_roll.rs` (2177 строк) — кода 1127, остальное тесты и
  комментарии; grab-bag сигнала нет.

Охват обхода: `cargo clippy --all-targets` (хост) + 12 синтаксических детекторов
`code_smell` по крейту `fretboard` (80 файлов, 1003 fn). Семантический слой `code_smell`
и аудиты `rust-code-mcp` не запускались.

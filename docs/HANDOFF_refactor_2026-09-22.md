# Хендофф: рефактор по `code_smell` — 2026-09-22, после Ф6б

Для новой сессии, которая продолжит [план рефактора](refactor_plan.md). План —
источник решений; этот файл — состояние на конец дня и разведка следующей фазы, чтобы
её не пришлось делать заново.

## ▶ Вход

1. Прочитать [refactor_plan.md](refactor_plan.md) целиком (разделы «Общие правила» и
   «Итог» у Ф6а/Ф6б обязательно) и «Landmines» в [violin_trainer_plan.md](violin_trainer_plan.md).
2. ~~Следующая фаза — Ф6в~~ — **✅ сделана в тот же день** («refactor(types): PNote для целых
   нот и один BankRange»); итог и решения — в плане. Разведка ниже оставлена как история.
3. Следующая — **Ф7** (снять теперь уже 23 `pub use` из `audio/mod.rs`, 14 файлов-импортёров).
   Она последняя по замыслу. Базовый счётчик после Ф6в: lib **195 passed / 15 ignored**.

## Состояние

Дерево чистое, ветка `main`. Коммиты дня, по порядку (SHA не пишем — см.
`memory/doc_shas_died_in_history_rewrite.md`, ищите по теме):

| Фаза | Тема коммита |
|---|---|
| план | docs(plan): план рефактора по обходу code_smell — 7 фаз с принятыми решениями |
| Ф1 | refactor(audio): отравленный мьютекс = паника, а не тихий дефолт |
| Ф2 | refactor(audio): один цикл аудио-воркера вместо двух копий |
| Ф3 | refactor(ui): impl Mark for Scale переезжает к трейту — core_types без egui |
| Ф4 | refactor(audio): распил native/imp.rs — drone, workers, capture, output |
| Ф5 | chore: total_cmp в ранжировании гамм, мёртвая egui из workspace |
| Ф6а | refactor(types): Hz и Midi — одна конверсия частота↔высота |
| Ф6б | refactor(audio): SampleRate — частота дискретизации отдельным типом |

**Базовый счётчик тестов** (`cargo test --release --lib --bins --tests`, после Ф6б):
lib **191 passed / 15 ignored**, `tests/pill_layout.rs` 2 passed, бинарники 0.
`release_ghosts_are_written_after_a_note` иногда флейкает (примерно 1 прогон из 5): одиночное падение
перегнать, регрессией не считать (подробности — «Попутные находки» плана).

## Гейт (все четыре, охват называть в коммите)

```
cargo clippy --all-targets                                   # 0 предупреждений кода; 3 манифестных (kebab-case бинарей) — норма
cargo check --target wasm32-unknown-unknown --lib --bins     # НЕ --all-targets: падает всегда
cargo check --target aarch64-linux-android --lib
cargo test --release --lib --bins --tests
rustfmt --edition 2024 <изменённые файлы>                    # без --edition — см. ловушки
code_smell prim --mode all --top 500 > <scratchpad>/prim_{before,after}.txt   # дельта, до и после
```

## Разведка Ф6в (сверено с кодом 2026-09-22, после Ф6б)

### Посылки плана, которые код опроверг

1. **«`serde(transparent)` на `PNote`, если его ещё нет»**: у `PNote` уже есть serde, и сделана она строже:
   `#[serde(try_from = "u8", into = "u8")]` (`core_types/pitch.rs:78-80`). На проводе и
   в RON это тот же `u8`, но с проверкой диапазона при чтении. Трогать не нужно.
2. **«`ResonatorViewSettings.min_midi/max_midi` → `BankRange` … проверить формат тестом
   совместимости»**: `ResonatorViewSettings` (`audio/dsp/resonator.rs:63-65`,
   `usize`) **не сериализуется**. Её собирают из персистентных
   `ResonatorSettings.min_midi/max_midi`, и они **уже `PNote`** (`audio/types.rs:483-484`,
   переход на `usize` — в `resonator.rs:150-151`). Если заменить поле вида на `BankRange`,
   формат на диске не изменится, так что тест совместимости не нужен. Менять
   `ResonatorSettings` на вложенный `BankRange` план не велит: это сменило бы RON
   (литерал держит тест в `types.rs:864`). Не делать без решения хозяина.
3. **Инвариант `lo <= hi` для `BankRange::new`** уже гарантирован источником:
   `ResonatorSettings::sanitized` (`types.rs:603-614`) держит `max ≥ min + 6`. `assert!` в
   конструкторе — это защита на границе, а не новая проверка.

### Сайты (свежие номера строк)

- **Две `BankRange`** (их снести, одну завести в `audio/dsp/resonator.rs`):
  - `app/staff_panel.rs:598` — `{ min_midi: i32, max_midi: i32 }`, `semitones()`, литерал
    в `:223`; комментарий «вырожденный диапазон» в `:604-605` переписать, как велит план.
  - `app/take_roll.rs:230` — `{ lo: f32, hi: f32 }`, `span()`, литерал в `:1015`, и
    **`const BANK: BankRange = BankRange { lo: 43.0, hi: 96.0 }` в тесте `:1153`**.
    `PNote::new` не `const fn` и возвращает `Option`, поэтому константу придётся сделать
    функцией `fn bank() -> BankRange`.
- **Регулярка критерия готовности слепа к выровненным полям.** `rg -n 'min_midi: (usize|i32)|max_midi: (usize|i32)' src`
  находит 13 строк, но **не видит `resonator.rs:64-65`** (`min_midi:          usize` —
  `struct_field_align_threshold` из `rustfmt.toml` ставит там пачку пробелов). Мерить
  критерий через `'min_midi:\s+(usize|i32)|max_midi:\s+(usize|i32)'` — **15 строк** на сейчас.
  Состав:
  - `ui/pianoroll.rs` — `res_min_midi/res_max_midi: i32` в трёх функциях (`:279-280`,
    `:406-407`, `:587-588`; регулярка не якорена, поэтому `res_` попадает) → на тот же
    `BankRange`;
  - `app/staff_panel.rs:599-600` — поля локальной `BankRange` (уходят вместе с ней);
  - `audio/dsp/resonator.rs:64-65` — `ResonatorViewSettings` (план: → `BankRange`);
  - параметры сетки спектра `scale_detect/chroma.rs:32,40,57`,
    `scale_detect/method_root.rs:27`, `dsp/analysis_math.rs:168` (`resonator_note_labels`,
    там и `max_midi`). `min_midi` здесь — начало `usize`-сетки (`NOTE_BUCKET_MIN_MIDI`),
    то есть целая нота; план эти места перечисляет.
- `ui/staff.rs:284`, `:297`, `:418`; `app/staff_panel.rs:749` (тест); `dsp/segmenter.rs:64`
  (`HeldNote::midi: i32`), `:178`, `:190` — целые ноты.
- **Не в списке плана, но тот же домен:** `StaffNote::midi: i32` (`audio/types.rs:64`) —
  публичный тип для панелей. План его не называет, поэтому трогать его можно только вместе
  с сайтами, которые его читают (`staff.rs`). Решить на месте и записать в «Итог».
- Арифметика со знаком (`midi - 12`, `midi + k`) — через `Interval`, как в `PNote::add`
  (план). Перед началом посмотреть, что `Interval` уже умеет.

## Ловушки, пойманные сегодня

- **Голый `rustfmt` на этом репо** парсит файл как 2015, падает на let-chains, и
  **`--check` не печатает ни одного `Diff in`**: выглядит как «всё отформатировано».
  Только `rustfmt --edition 2024`. `native/imp.rs` — корень `imp/*.rs`, rustfmt проходит и по детям.
- **`memory/` в `.gitignore`** (`.gitignore:9`). Если путь `memory/…` попадёт в
  `git commit -- <пути>`, **весь коммит падает** с `pathspec did not match`. Память правится
  и не коммитится.
- **Правки только через Edit/Write.** Сегодня два раза прошли `sed -i` — это против политики
  репо, так не делать.
- **Пример в тесте проверять запуском.** Первый тест `SampleRate::samples_in` утверждал, что
  `48000 × 0.03f32 < 1440`. Это неверно: f32 округляет ровно в 1440. Поймал clippy
  (`assertions_on_constants`). Настоящий свидетель: 48 кГц × 9 мс = 431.99997.

## Ждут решения хозяина (не механика фаз)

Все записаны в плане с местом в коде:

- `native/imp.rs` = 1038 строк при пороге Ф4 `< 1000` — выносить ли методы сборки
  захвата в `capture.rs` (Ф4, «Итог»).
- `current_input_sample_rate()` отдаёт `0` до старта захвата, а UI печатает «Input rate: 0 Hz»
  (`app/controls.rs:330`). Сделать `Option<SampleRate>` = решить, что показывать (Ф6б,
  «Попутные находки»).
- `TunerReading::fast_pitch/melody_pitch` никто не читает — снести или найти потребителя.
- `frequency_hz: f32 ×5` (частоты детектора) — добить ли типом `Hz`.
- `ActiveCapture.selected_id = ""` при реплее без выбранного входа.

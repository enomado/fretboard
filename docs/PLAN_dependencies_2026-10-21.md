# Fretboard: зависимости → code smells — 2026-10-21

Запрос Стаса: один fleet-трек, сначала 6 сессий на свежие зависимости,
включая egui и egui_dockyard, если используется, затем 8 на code smells.
Единая серия `fretboard-deps`, общая очередь из 14 областей и лимит
14 сессий, включая уже начатую первую. Рабочая ветка `main`.
Отдельная серия `fretboard-code-smell` остановлена, её очередь перенесена сюда.

## Очередь

Один назначенный пункт — область одной сессии. Незаконченный пункт передать
следующей сессии; не закрывать заранее и не запускать вложенные серии.

<!-- AUTORUN_TASKS_START -->
- [x] dependency-inventory — проверить все прямые, целевые, dev- и git-зависимости, доступные релизы и совместимость; снять исходные проверки и записать короткий список обновлений/осознанных исключений.
- [ ] egui-upgrade — согласованно обновить eframe/egui, egui_tiles, egui_kittest и граф рендеринга до свежих совместимых версий, мигрировать используемые API, проверить desktop и UI-тесты.
- [ ] audio-upgrade — обновить cpal, ringbuf, rustfft, realfft, resonators и остальные аудиозависимости; сохранить расчётные контракты, проверить затронутый DSP и нативное аудио.
- [ ] platform-upgrade — обновить wasm-bindgen/js-sys/web-sys, Android/JNI и hot-reload зависимости, восстановить web/Android/desktop пути при изменении API.
- [ ] remaining-dependencies — обновить остальные прямые и транзитивные зависимости и Cargo.lock; повторно проверить свежесть и отсутствие ненужных дублей egui; разобрать standalone probe с учётом его назначения.
- [ ] upgrade-validation — проверить итоговый граф на desktop, wasm и Android, прогнать подходящие тесты и rustdoc; исправить регрессии, записать итоговые версии, реальные исключения и остаток.
- [ ] smell-baseline — после завершения upgrade-validation выполнить MCP-аудиты, clippy и code_smell stats/каталог/применимые детекторы, собрать адресный backlog с приоритетами и известными FP.
- [ ] domain-types — разобрать primitive obsession, голые индексы/единицы и половинчатые миграции; использовать существующие канонические типы и проводить тип через внутренние вызовы.
- [ ] error-contracts — разобрать fallback-затычки, Option, NaN и внешние ошибки; исправить подтверждённые нарушения контрактов без подавления диагностики.
- [ ] reuse-and-boundaries — разобрать дубли, feature envy, длинные цепочки, data/arg clumps и циклы модулей; колокализовать инварианты, переиспользуя существующее.
- [ ] audio-code-smells — разобрать подтверждённые smells native/wasm capture, workers, replay и DSP; сохранить аудиочасы, численные алгоритмы и realtime-свойства.
- [ ] ui-code-smells — разобрать применимые egui-детекторы, состояние панелей, диапазоны и ввод; сохранить поведение, проверить изменённую интеракцию egui_kittest.
- [ ] remaining-findings — исправить оставшиеся приоритетные подтверждённые находки baseline, проверить policy-check и устаревшие allowlist при наличии политики; не изобретать работу ради счётчика.
- [ ] smell-validation — повторить применимые детекторы с сопоставимым охватом, проверить изменения на desktop/wasm/Android, зафиксировать дельту, FP и конкретный оставшийся backlog.
<!-- AUTORUN_TASKS_END -->

Очередь строго последовательная: smell не начинать до завершения всех шести
областей обновления, включая upgrade-validation. Лимит сессий не заменяет
проверку завершения: если обновление требует продолжения, следующий ход
продолжает его, а smell остаётся в очереди.

## Границы и проверки

Перед правками прочитать AGENTS/CLAUDE, проектные дополнения, относящиеся
документы и `memory/module_layout_and_target_gates.md`. Обновлять зависимости
по фактическим релизам registry/upstream на момент работы. Начальная версия
eframe — 0.36.2; egui 0.37.0 опубликован:
[релизы egui](https://github.com/emilk/egui/releases).
В корневом Cargo.toml используется egui_tiles; egui_dockyard не объявлен.
Проверить весь граф: если dockyard отсутствует, не добавлять новый docking
движок ради обновления. Если обнаружится — обновить существующую зависимость.
Не переносить в fretboard вендор и форки bur.

Обновления major разрешены вместе с необходимой миграцией API. Для связки
egui выбирать совместимый граф, причину удержанного пакета записывать явно.
Не менять алгоритмы детектора, продуктовый UI и форматы данных попутно.
Не ослаблять тесты и численные эталоны. `resonators` намеренно следует ветке
`onepole-bank`: учитывать комментарий манифеста, не заменять форк старым crates.io.
`probes/lifecycle_probe` — самостоятельный воспроизводимый upstream-репро:
прочитать его README и оценить, допустимо ли обновление закреплённых версий;
если разрушает исходное измерение — оставить с причиной, без переписывания истории.

Итоговый охват:
- desktop: `cargo clippy --all-targets`, подходящие lib/integration/UI-тесты;
- wasm: `cargo check --target wasm32-unknown-unknown --lib --bins`;
- Android: `cargo check --target aarch64-linux-android --lib` с доступным NDK;
- rustdoc: `RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --lib`, цели по затронутым API.

Wasm `--all-targets` не является гейтом: dev-dep egui_kittest/wgpu нативный.
Проверки выбирать по изменению; полный проход — после интеграции.
Называть реально выполненные цели/тесты. Недоступный SDK — ограничение проверки,
а не доказательство зелёной сборки. Рабочее окно Стаса не перезапускать.
Коммитить только свои пути, не пушить, чужую `.claude/` не включать.

## Code smells

Предыдущий рефактор Ф1–Ф7 в `docs/refactor_plan.md` завершён: не повторять.
Прочитать локальный skill code-smell и README установленного инструмента.
Порядок: scoped MCP health/audit → clippy → code_smell, первой командой
суиты — `stats`. Проверять фактический каталог, включая egui и доступные
compiler/semantic движки. Exit 2, отсутствие политики и unresolved —
ограничения проверки, а не pass. До семантического поиска использовать MCP.
Настраивать политику только по реальным доменам проекта, с объяснением
исключений; не добавлять произвольные allowlist ради чистого отчёта.

Исходный вывод сохранить вне git в состоянии единой серии; итог оценивать
по дельте и фактическому охвату. Кандидатов подтверждать чтением контрактов,
FP объяснять (trait signatures, численные clamps, domain bundles и т.д.).
Полезные находки исправлять, не ограничиваясь отчётом и не изобретая работу
ради счётчика. Единый backlog и текущий хендоф — раздел ниже.

Сохранять locked-решения `docs/note_detection.md` §7 и памяти проекта.
Не менять DSP-поведение, численные эталоны, продуктовые решения или старые
форматы ради smell. H3/H7 и снос ремонтного слоя ждут Стаса;
новые настройки и переключатели не добавлять. Для изменённой UI-интеракции
использовать существующий egui_kittest, state-тест не заменяет проверку ввода.
Форматировать только изменённые файлы. Вложенные серии и субагенты не нужны.
Находки вне скоупа записывать в backlog с адресом и причиной.

## Инвентаризация зависимостей — 2026-10-21

Проверены оба манифеста и lockfile, workspace/target/dev-разделы: в приложении
27 прямых имён (26 registry и один git), build-dependencies нет; в standalone
probe три registry-зависимости. Исходный корневой lockfile содержит 503 пакета.
`cargo metadata --locked` и `cargo tree --locked --target all --duplicates`
подтвердили единственный egui 0.36.2 и отсутствие dockyard/другой docking-библиотеки.
Свежесть проверена через live crates.io API для каждого прямого имени;
ниже текущие **locked** версии и последние стабильные релизы, не только требования
манифеста. Корневые Cargo.toml/Cargo.lock в этой сессии не обновлялись.

| Область | Зависимости: исходная → доступная версия |
|---|---|
| egui-upgrade | [eframe](https://crates.io/api/v1/crates/eframe), egui, egui_kittest и связка epaint/egui-wgpu/egui-winit/egui_glow: 0.36.2 → 0.37.0; [egui_tiles](https://crates.io/api/v1/crates/egui_tiles): 0.17.1 → 0.18.0 |
| audio-upgrade | [ringbuf](https://crates.io/api/v1/crates/ringbuf): 0.5.2 → 0.5.3 |
| platform-upgrade | [wasm-bindgen](https://crates.io/api/v1/crates/wasm-bindgen): 0.2.128 → 0.2.129; wasm-bindgen-futures: 0.4.78 → 0.4.79; js-sys/web-sys: 0.3.105 → 0.3.106 |
| remaining-dependencies | [serde_json](https://crates.io/api/v1/crates/serde_json): 1.0.151 → 1.0.154; обновить совместимые транзитивные версии в Cargo.lock |

Остальные 17 registry-зависимостей уже на последних стабильных версиях:

| Назначение | Locked = последний stable |
|---|---|
| Общие | itertools 0.15.0, nom 8.0.0, serde 1.0.229, web-time 1.1.0 |
| DSP/native | rustfft 6.4.1, realfft 3.5.0, cpal 0.18.2, hound 3.5.1 |
| Dev | ron 0.12.2, image 0.25.10 |
| Desktop hot reload | subsecond 0.7.10, dioxus-cli-config 0.7.10, tungstenite 0.30.0 |
| Android | android-activity 0.6.1, jni 0.22.4, ndk-context 0.1.1 |
| Wasm wire format | postcard 1.1.3 |

Совместимость и осознанные исключения:

- В собственной временной копии манифеста и lockfile реально выполнен
  `cargo update -p eframe -p egui_tiles -p egui_kittest`: набор 0.37.0/0.18.0
  разрешается с одним egui. Затем полный `cargo update` нашёл ещё 43 обновляемых
  пакета; это проверка разрешения, API приложения с новыми версиями ещё не собирался.
  wgpu 30.0.1 и winit 0.30.13 остаются последними стабильными и совместимыми;
  принудительно переводить winit на 0.31.0-beta.3 не нужно.
  MSRV новой egui-связки 1.95, установлен rustc 1.101.0-nightly (2026-10-04).
- [eframe 0.37](https://github.com/emilk/egui/blob/0.37.0/crates/eframe/CHANGELOG.md)
  перестал безусловно включать `egui-winit/clipboard`: при наших
  `default-features = false` добавить desktop feature `clipboard`, сохраняя
  копирование/вставку. Этот вариант графа тоже успешно разрешён в scratch.
  `default_fonts` уже включён явно. Для миграции проверить
  `src/app/workspace.rs` (Behavior/Panel), `src/ui/theme.rs` (fonts) и
  `tests/pill_layout.rs`: в kittest 0.37 строже проверяются glyph/accessibility,
  появился `with_context_setup`. Тесты и пиксельные ожидания не ослаблять.
- wasm-bindgen-семейство обновлять вместе: js-sys 0.3.106 и futures 0.4.79
  требуют **=0.2.129**, web-sys требует **=0.3.106** js-sys. Новый eframe
  допускает этот набор. realfft 3.5.0 требует rustfft ^6.4.0 и сохраняет
  единственный num-complex 0.4.6. Android activity/cpal сходятся на jni 0.22.4.
- [resonators PR #6](https://github.com/jhartquist/resonators/pull/6) открыт,
  не merged. Ветка onepole-bank и Cargo.lock совпадают на `67152aa9`;
  crates.io всё ещё 0.1.1, upstream HEAD уже объявляет неопубликованную 0.1.2.
  Сохранить наш fork/ветку и lock: опубликованная версия не заменяет OnePoleBank
  и f64 phase fix. На момент проверки нового коммита ветки для обновления нет.
- subsecond/dioxus-cli-config 0.8.0-alpha.3 — prerelease; оставить согласованную
  стабильную пару 0.7.10. Транзитивные дубли вроде itertools 0.14 через rav1e,
  jni-sys 0.3/0.4 через ndk и платформенные objc2 не устранять прямыми пинами.
- lifecycle_probe сохранить с обоими исходными Cargo-файлами: все три прямые
  версии уже последние, а обновление остальных пакетов изменит окружение
  измерения android-activity#174. В README добавлены роль lockfile и проверенная
  locked-команда; `cargo-apk apk run --help` не предлагает флаг `--locked`.

Вспомогательные инструменты тоже просмотрены: npm-манифеста нет, CJS используют
Node builtins и сохранённый aubio WASM; Python имеет PEP 723-зависимости, JVM —
явные версии в `tools/tuner_app_probe.py`. Последние stable проверены через
[PyPI JSON](https://pypi.org/pypi/swift-f0/json) и
[Maven metadata](https://repo.maven.apache.org/maven2/org/jetbrains/kotlin/kotlin-stdlib/maven-metadata.xml):

- SwiftF0 0.1.2 → 0.3.0 сознательно **не обновлять** в oracle/lag probe:
  это другой внешний эталон. Сохранить также pinned pitch-benchmark commit,
  Tuner commit и aubio WASM; пересоздание численных эталонов не входит в серию.
- Для незакреплённых Python support-пакетов доступны numpy 2.5.4, pandas 3.0.6,
  scipy 1.18.1, torch 2.14.1, torchaudio 2.11.0, torchcodec 0.17.0,
  soundfile 0.14.0, librosa 1.0.0, numba 0.68.0, tqdm 4.70.1,
  onnx 1.23.2, onnxruntime 1.31.0, matplotlib 3.11.2.
  `uv lock --script <script> --python 3.12 --no-python-downloads --dry-run --no-build`
  успешно разрешил swiftf0_oracle (83 пакета), lag_sweep_probe (76) и
  pitch_roll_plot (18), учитывая весь заявленный Python-диапазон.
  Wheels не устанавливались, runtime/ABI и пересчёт корпусов не проверялись.
- JVM reference harness удержать: Kotlin/compiler/stdlib 2.4.10 (доступен 2.4.21),
  kotlin-reflect 1.6.10 (2.4.21), coroutines 1.8.0 (1.11.0), annotations 13.0
  (26.1.0). Это инструменты воспроизведения сохранённого Tuner, не runtime
  fretboard; обновлять их вместе с внешним эталоном отдельной предметной задачей.

Исходные проверки, все на старом графе, все cargo-команды с `--locked`:

- `cargo clippy --all-targets` — pass с одним существующим warning
  `chunks_exact_to_as_chunks` в `src/audio/dsp/sieve.rs:907` (lib test).
- `cargo test --all-targets` — 242 lib + 2 UI passed, 0 failed, 36 ignored;
  пять bin harness собраны, в них 0 тестов. Ignored/корпусные замеры не запускались.
- `cargo check --target wasm32-unknown-unknown --lib --bins` — pass.
- `cargo check --target aarch64-linux-android --lib` — pass с NDK r27c,
  `ANDROID_NDK_ROOT`, `CC_aarch64_linux_android` (clang API 26) и
  `AR_aarch64_linux_android` (llvm-ar). Та же команда с
  `--manifest-path probes/lifecycle_probe/Cargo.toml` — pass.
- `RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --lib` — pass.
  APK/браузер/физическое аудио не запускались; итоговые проверки нового графа
  по-прежнему обязательны в upgrade-validation.

## Текущее состояние

Сессия 1: dependency-inventory завершён; блокеров нет. Следующая область —
egui-upgrade: применить набор 0.37.0/tiles 0.18.0 с desktop clipboard,
мигрировать API и проверить desktop/UI. Аудио, platform и остальные обновления
остаются своим следующим сессиям; корневой граф пока исходный.
Сохранены все 14 пунктов единой очереди; smell не начинался и ждёт
upgrade-validation. Чужая untracked `.claude/` не затронута.

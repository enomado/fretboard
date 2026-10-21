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
- [x] egui-upgrade — согласованно обновить eframe/egui, egui_tiles, egui_kittest и граф рендеринга до свежих совместимых версий, мигрировать используемые API, проверить desktop и UI-тесты.
- [x] audio-upgrade — обновить cpal, ringbuf, rustfft, realfft, resonators и остальные аудиозависимости; сохранить расчётные контракты, проверить затронутый DSP и нативное аудио.
- [x] platform-upgrade — обновить wasm-bindgen/js-sys/web-sys, Android/JNI и hot-reload зависимости, восстановить web/Android/desktop пути при изменении API.
- [x] remaining-dependencies — обновить остальные прямые и транзитивные зависимости и Cargo.lock; повторно проверить свежесть и отсутствие ненужных дублей egui; разобрать standalone probe с учётом его назначения.
- [x] upgrade-validation — проверить итоговый граф на desktop, wasm и Android, прогнать подходящие тесты и rustdoc; исправить регрессии, записать итоговые версии, реальные исключения и остаток.
- [x] smell-baseline — после завершения upgrade-validation выполнить MCP-аудиты, clippy и code_smell stats/каталог/применимые детекторы, собрать адресный backlog с приоритетами и известными FP.
- [x] domain-types — разобрать primitive obsession, голые индексы/единицы и половинчатые миграции; использовать существующие канонические типы и проводить тип через внутренние вызовы.
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

## Итог обновлений (сессия 6)

Сессия 6: upgrade-validation завершён, все шесть областей обновления закрыты;
блокеров и выявленных регрессий нет. Исходники и lockfile в этой сессии
не потребовали изменений. На 2026-10-21 повторно проверены live
[crates.io API](https://crates.io/api/v1/crates/eframe) всех 26 прямых
registry-зависимостей (три имени standalone probe входят в этот набор),
а также egui, wgpu и winit. Все прямые registry-зависимости приложения —
последние stable. Итоговые locked-версии:

| Область | Версии |
|---|---|
| UI/рендеринг | eframe/egui/egui_kittest 0.37.0, egui_tiles 0.18.0, wgpu 30.0.1, winit 0.30.13 |
| DSP/native | rustfft 6.4.1, realfft 3.5.0, cpal 0.18.2, ringbuf 0.5.3, hound 3.5.1 |
| Общие | itertools 0.15.0, nom 8.0.0, serde 1.0.229, serde_json 1.0.154, web-time 1.1.0 |
| Desktop hot reload | subsecond/dioxus-cli-config 0.7.10, tungstenite 0.30.0 |
| Android | android-activity 0.6.1, jni 0.22.4, ndk-context 0.1.1 |
| Wasm | wasm-bindgen 0.2.129, wasm-bindgen-futures 0.4.79, js-sys/web-sys 0.3.106, postcard 1.1.3 |
| Остальные dev | ron 0.12.2, image 0.25.10 |

`cargo metadata --locked`, `cargo tree --locked --target all --duplicates`
и обратное дерево egui подтвердили 502 пакета, единственный egui 0.37.0
и согласованный UI-граф; dockyard отсутствует. Повторный
`cargo update --dry-run --verbose` с обновлением registry/git — 0 изменений.

Реальные исключения:

- resonators 0.1.1 — намеренный fork onepole-bank с OnePoleBank/f64 phase fix;
  `git ls-remote` вновь подтвердил locked HEAD `67152aa9`. Crates.io не заменяет fork.
- subsecond/dioxus-cli-config 0.8.0-alpha.3 и winit 0.31.0-beta.3 — prerelease;
  сохраняются последние stable из таблицы.
- 118 транзитивных записей ниже latest stable разобраны в сессии 5 по registry
  index и resolved-рёбрам: несовместимые major исключены требованиями родителей.
  Например, eframe/egui_glow/wgpu удерживают glow ^0.17; epaint — harfrust ^0.12,
  skrifa ^0.44, vello_cpu ^0.2; accesskit/atspi — zbus ^5; cpal — alsa ^0.11.
  Дубли image/rav1e, proc-macro и платформенных цепочек требуют миграции upstream;
  прямых пинов или дополнительных форков ради их удаления нет.
- lifecycle_probe сохраняет оба исходных Cargo-файла и окружение MIUI-замера
  android-activity#174; его три прямые версии уже последние stable.
- Внешние численные эталоны, SwiftF0 0.1.2 и JVM/reference harness сохранены
  по причинам из инвентаризации; их обновление подменило бы исходное измерение.

Реально выполнено на окончательном графе (cargo с `--locked`):

- `cargo test --all-targets` — 244 lib + 2 UI passed, 0 failed, 36 ignored;
  пять bin harness собраны, в них 0 тестов. Включены egui_kittest pixel-тесты,
  RON round-trip и чтение layout tiles 0.17.1. Численные эталоны не изменялись;
  ignored/корпусные замеры не запускались.
- `cargo clippy --all-targets` — pass с прежним warning
  `chunks_exact_to_as_chunks` в `src/audio/dsp/sieve.rs:907`.
- `cargo build --bin fretboard-app` — pass.
- `cargo check --target wasm32-unknown-unknown --lib --bins` — pass.
- `cargo check --target aarch64-linux-android --lib` — pass для приложения
  и отдельно `--manifest-path probes/lifecycle_probe/Cargo.toml`;
  NDK r27c, CC clang API 26, AR llvm-ar. Probe lockfile не изменился.
- `RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --lib` — pass для desktop,
  с `--target wasm32-unknown-unknown` и с `--target aarch64-linux-android`.
- Trunk 0.21.14 `build --locked --public-url /upgrade-s6/` — pass для
  fretboard-app и dsp_worker, wasm-bindgen CLI 0.2.129. Как и в сессии 4,
  использован `env -u NO_COLOR` из-за внешнего `NO_COLOR=1`.
- Отдельный headless Chromium: sub-path, canvas и один DSP worker загрузились;
  Ready получен, на Init(48000) и 4096 синтетических сэмплов пришёл Snapshot
  8204 байта. JS-исключений, worker errors и ошибок загрузки бандла нет.
  Микрофон/getDisplayMedia, физическое аудио, APK/устройство и применение
  hot-reload patch этим проходом не проверялись; окно Стаса не запускалось.
- `git diff --check` — pass.

Остатка по обновлениям нет. Smell-аудит в сессии 6 не начинался.

## Текущее состояние — хендоф сессии 8

`domain-types` завершён. Следующая область — **error-contracts**, начиная с
P1 в `Declaration`; её исправление здесь не начиналось. Блокеров нет.
Исходный baseline сохранён в `s7-baseline/`, результаты этой сессии —
`/home/sc/t/scratch/fretboard_deps_codex_autorun/s8-domain-types/`.

Проведены существующие типы: `Hz` от выхода pYIN через `PitchEstimate`, held
frequency, `TunerReading` и `frequency_to_note` до `TunerTarget`; `Midi` через
`PitchGrid`, начала salience-сеток всех фронтендов, `SalienceHeat` и `HeatGrid`.
Границы виолончельных позиций теперь сразу `Fret`. `PitchState` и `BankRange`
сохранены; второй grid не введён. Формулы, интервалы, порядок операций,
sample clock и DSP-эталоны прежние. Числовые serde-поля сохранены transparent
типами; добавлен тест чтения и записи прежней формы snapshot.

Аудит: scoped MCP healthy/fresh, census 158 типов и pub-use audit 0;
clippy до и после чистый. `stats`: 95 src-файлов, 1222 fn, 162 struct,
1381 параметр, parse ERROR 0. `prim --mode all`: объявления примитивов
858→836, parameter leaks 8→7; dup-arity 43→46 — новые пары `Hz`/`Midi`/`Fret`
обозначают эталон/измерение и границы диапазонов, не требуют роль-типов.
Численные индексы одного буфера, fractional-bin арифметика, плотность сетки,
FFT-шаг и численные массивы внутри ядра не равны высоте/абсолютной частоте;
`input_kind` с уровнем, chroma с максимумом и диагностические tuple-sort keys
не подтверждают утечку одного домена. Allowlist не добавлялся.

Повторены egui Dylint range-order (45 Known, 2 OutsideCompiler test-only) и
range-flow (86 src + main, 58 HIR fn/104 sinks, 0 кандидатов, 4 outside-compiler,
0 params-unresolved). Semantic-сборка на отдельной копии без `.claude/`:
range-flow — 98 VFS-файлов, те же 58/104, 0 кандидатов/unresolved;
repack-flow high-signal — 95 файлов/273 constructions/0 source-target pairs.
Это узкие классы, не общий pass типов. `half-migration` снова exit 2:
библиотек алгебры нет, класс неприменим. Полный каталог — в разделе Smell-baseline.

Реально проверено на итоговом коде: `cargo test --locked --all-targets` —
**245 lib + 2 UI passed**, 36 ignored, пять пустых bin harness;
`cargo clippy --locked --all-targets`, wasm `--lib --bins`, Android `--lib`
(NDK 27.0.12077973, API 26) и desktop rustdoc с `-D warnings` — pass.
`policy-check` — 3 пути/96 файлов, 0 находок; `git diff --check` — pass.
UI-интеракция не менялась. Физическое аудио/Android-устройство, ignored probes
и браузерный runtime здесь не проверялись; общий финальный проход остаётся
за smell-validation. Locked §7, H3/H7 и ремонтный слой сохранены.

## Smell-baseline: охват, backlog и FP

- Scoped MCP healthy/fresh; гиперграф: 6 локальных targets, 1273 items.
  `workspace_stats`, `dead_pub_report` (182), `semantic_overlaps` (1273 seeds,
  4980 пар при 0.8, без усечения), `crate_dependency_metric`,
  `pub_use_pub_type_audit` (0), `unsafe_audit` (1), `mut_static_audit` (0),
  `channel_capacity_audit` (1), `derive_audit(Debug)` (24), `fn_body_audit`
  (107: 90 unwrap, 9 self-recursion, 8 panic) сохранены. Это desktop HIR;
  `pub` и derive — worklist, не разрешение массово сужать API/добавлять Debug.
- Затем обычный clippy; дополнительный проход с `too_many_lines`,
  `cognitive_complexity`, `excessive_nesting` дал 10 уникальных адресов только
  `too_many_lines` (15 сообщений с повторами lib/lib-test), не build-gate.
- Первая команда суиты — `stats`: 1 пакет fretboard, **95 src-файлов,
  1221 fn, 162 struct, 1381 параметр**, 0 файлов с parse ERROR.
  Выполнены `prim --mode all --top 10000 --sites 10`, `range-sub` (8),
  `god-file` (0 grab-bag/5 size-only), `dep-cycle` (1 local/0 cross-cut),
  `co-change` (124 из 249 коммитов; 150 cross-module/96 neighbor пар),
  `option-crutch --mode all` (34 fallback/3 error, 60 полей, 16 co-field
  структур), `frame-ledger` (7 кандидатов/0 после сита), `serial-await` (0).
  Syntax arg-clump: 16 params/7 fields; scan-loop: 9 кандидатов;
  nan-clamp: 5 domain/1 divisor/27 fold; nan-swallow: 4/4 предиката разобрано,
  0 swallow/order.
- Compiler/Dylint выполнены: **arg-clump, iter-chain, feature-envy,
  nan-clamp, nan-swallow, scan-loop, scan-rank, range-order, range-flow,
  key-read**. Основной охват — 86 src-файлов + main.rs; nan-clamp сообщает
  87 файлов. Только host lib/bins/default features, без test targets и
  wasm/Android cfg. Arg-clump: 720 fn/132 struct baskets, 10/6 клумпов;
  iter-chain: 16 сайтов/2 группы; feature-envy: 16 + 7 demoted, JSON помечен
  `pilot_only=true`, подтверждён RA. Nan-clamp: 4 domain/19 fold;
  nan-swallow: 4/4 предиката, 0 swallow/order. Scan-loop/rank: 6 сайтов.
  Range-order: 45 Known + **2 OutsideCompiler** (`analysis_math.rs:303`,
  `sieve.rs:491`, обе test-only функции). Range-flow: 58 HIR fn/104 sinks,
  0 кандидатов, **4 outside-compiler**, 0 params-unresolved. Key-read: 1 Known.
  Первые scan-loop/key-read отвергли изменившиеся тесты (exit 2); приняты
  только повторные `*-compiler-stable.json` с Ran, exit 0.
- Semantic-бинарь собран из установленного tool checkout с `--features semantic`.
  Первичный RA обход ошибочно включил вложенный чужой worktree: эти отчёты
  **не baseline**. Приняты повторные `scoped-*` на отдельной копии текущих
  src/tests/assets и Cargo-файлов без `.claude/`: stats те же 95 файлов,
  HIR обход 98 VFS-файлов/896 fn. Over-borrow 13; envy 16 + 7 demoted;
  cross 0; iter-chain 16 сайтов/2 группы; scan-rank 1 high/1 med/2 low.
  Repack-flow high-signal: 95 файлов/273 constructions/0 находок, это только
  узкая lossless форма. Option calls: 55 ret/49 param, 1 firm-кандидат;
  fields-semantic: 60 полей, **635 unresolved**, разобрано 101/2751 доступов
  и 67/906 полей в литералах — ноль tighten не оправдывает все поля.
  RA range-flow: 69 prefilter/58 HIR fn/104 sinks, 0 кандидатов/params-unresolved.
  Блока потерянных из-за panic тел нет. Копия исходников и сборка удалены;
  команды и исходные результаты сохранены.
- Egui дополнительно: ui-token 164 literal (49 token-home, 13 theme, 102 domain)
  + 19 derived; key-read syntax/compiler согласны. Test-no-assert: **96 файлов,
  282 теста**; до фикса 259 asserts/21 panic-only/2 mute, после 260/20/2;
  5676 unresolved вызовов после фикса — это не сертификат полноты.
  `calc-in-panel` без политики дал exit 2; временный override DSP-дома
  на одном пакете дал **0 индексированных файлов/имён**, хотя exit 0.
  Инструмент ищет межкрейтовые вызовы, поэтому ноль класса A не засчитан.
  Класс B (10 функций) — только worklist геометрии/панелей.
- `half-migration` — exit 2: нет пакетов с библиотекой алгебры, класс неприменим.
  Grid-text-color не имеет канонического helper/общего токена сетки fretboard;
  SQL, MD/геокоординаты, Project/undo-mutations и пара computed/live model
  не являются доменами этого приложения. Соответствующие policy-детекторы
  не покрыты; чужие имена/политику не заимствовали. Отсутствие политики,
  exit 2, OutsideCompiler и unresolved нигде не объявлены pass.

### Адресный backlog

P1 — подтверждённая ошибка внешнего ввода; P2 — полезная мишень/проверка
контракта, ещё не приговор; P3 — низкий приоритет. Порядок областей прежний.

| Область / приоритет | Адрес и следующий конкретный разбор |
|---|---|
| error-contracts / **P1** | `app/take_marks.rs:88,104` → `note.rs:232` → `pitch.rs:15`: Declaration принимает `Cb4/E#4/Fb4/B#4/C10`, затем `.midi()` паникует; `C21/G255` молча дают MIDI **8/7** из-за `as u8`. Подтверждено исполняемым `declaration-boundary-probe.log` на неизменённом коде. Нужна валидация пользовательской границы до создания Declaration; расширение поддерживаемой энгармоники не решать попутно. |
| error-contracts / P2 | `audio/worker_proto.rs:63–70`: generic encode гасит ошибку сериализации пустым пакетом; комментарий про callback не доказывает контракт всех callers. Проверить worker/main обработку и сохранить причину ошибки. `bin/mic_probe.rs:285`: join panic превращён в пустые данные. |
| error-contracts / P2 | `app/persist.rs:40,84`: wire `test_note_midi: usize` сужается через `as u8` до проверки `PNote::new`; 256 становится законным MIDI 0 вопреки комментарию fail-fast. Проверить загрузку повреждённого/старого RON, сохраняя действующую политику восстановления настроек. |
| reuse-and-boundaries / P2 | `app.rs:365,401,407,413`, `app/controls.rs:59,63,111`: 7 повторных поисков `AudioInputOption` по id/kind, 2 группы обоих движков. Общая навигация должна сохранять приоритет preferred/fallback и реальное отсутствие устройства. |
| reuse-and-boundaries / P3 | `audio/core.rs:307,498`: идентичные push_samples-обёртки; `native/imp/devices.rs:324` и `bin/mic_probe.rs:403`: повтор device label. Обёртки разных pipeline не требуют объединения DSP; оценить только полезный общий контракт. Local cycle `core_types::note ↔ pitch` низкоприоритетный, ожидаем для представлений одной ноты. |
| audio-code-smells / P2 | `native/imp.rs:239,361–480`: unbounded очередь команд при медленном открытии устройства; проверить поток UI→control thread и способ ограничения/коалесцирования без блокирования UI. `core.rs:307,498`/arg-clump AudioContext — учитывать владение/порядок остановки. |
| audio-code-smells / P2 | `dsp/rtswipe.rs:499`, `sieve.rs:425`, `swipe.rs:612`, `trellis.rs:320`: 4 compiler domain NaN clamps, плюс 19 fold. Прочитать finite/nonnegative контракты источников; не менять sqrt/log/likelihood и эталоны ради счётчика. |
| ui-code-smells / P2 | `ui/pianoroll.rs:193`, `app/take_roll.rs:537`: вычисляемые Rect при узком plot и клиппинге marks; проверить реальные min/max и ввод через egui_kittest. `controls.rs:44,646`, `ui/snail.rs:64`: длинные UI-функции/over-borrow; выносить только связный инвариант, не набор виджетов ради LOC. |
| remaining-findings / P2 | `src/main.rs:48`: единственный desktop unsafe без объяснения доверия к jump table; прочитать subsecond apply_patch Safety и контракт devserver, не сочинять гарантию корректности указателей по одному PID. MCP 182 dead-pub/24 missing Debug — только выборочный разбор реальных потребителей. |
| smell-validation | Повторить принятые движки с теми же flags/охватом; RA из чистой копии без вложенных деревьев. Держать unresolved/неактивный cfg отдельными ограничениями, сопоставлять `.exit` и JSON verdict, затем выполнить итоговые target-гейты выше. |

### Уже разобранные FP и неизменяемые решения

- `MelodyHistory::oldest` (`audio/types.rs:258`) **не ужесточать** по единственному
  unwrap caller: пустая/очищенная история законна, `.front()` действительно None.
  `Declaration::parse(...).ok()` законен как «ещё не нота», но это не оправдание
  P1 отсутствия проверки переводимости принятой ноты в MIDI.
- `workers.rs:145` — **полный** `batch.drain(..)`, хвоста для сдвига нет;
  это не MidCut O(N²). Поиск pYIN ограничен `N_THRESHOLDS`, что видно в коде,
  а не доказывается эвристикой «Vec::new ⇒ константа». Строковые contains
  в классификации устройств перебирают фиксированный набор маркеров.
  Реальный high-кандидат `take_panel.rs:177` (corpus × takes) требует
  scaling-проверки до оптимизации, не выдан за измеренную проблему.
- Arg-clump signatures `egui_tiles::Behavior` определены внешним трейтом;
  `ANote/Accidental/Octave`, `RollMapping`, wire `Latest/WorkerSnapshot` —
  доменные бандлы/граница, не повод слить независимые модели.
  Envy рисовалки к DTO не переносится в `core_types`/DSP; читать как вопрос
  сужения UI borrow. MCP 9 «self-recursion» — совпадения имён методов разных
  типов (`new`, `name`, `sanitized`, делегирование tracked_bins), не рекурсия.
- `take_roll.rs:976` Escape отменяет незавершённую отметку, второго читателя
  в src нет; конфликт не подтверждён, для изменения фокуса нужен kittest.
  `take_roll.rs:620,657` заранее делают `whole.max(MIN_...)`: порядок clamp
  обеспечен кодом. Цвета спектра/высоты и локальные const разрешены `tokens.rs`.
  Два MUTE и большинство PANIC-ONLY — диагностические DSP/preview probes;
  не придумывать численный эталон вместо их роли.
- Locked §7, H3/H7, ремонтный слой, sample clock, две staff-линейки и
  независимость pitch/heat сохранены. Чужая `.claude/` не изменялась.

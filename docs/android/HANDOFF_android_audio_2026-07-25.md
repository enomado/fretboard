# Хендофф — Android-аудио + апстрим-сага mic-permission (2026-07-25)

Точка входа для новой сессии. Предыдущий слой контекста —
[KICKOFF_android_snail_build.md](KICKOFF_android_snail_build.md) (грабли сборки, пермишен,
хронология апстрима). Здесь: что измерено на устройстве сегодня, что осталось починить,
и все микрорешения, чтобы их не принимать заново.

## Итог сессии в одну строку
Звук на Android **работает** на HEAD (измерено на устройстве); апстрим-трек
mic-permission закрыт с нашей стороны (PR смёржен, замеры и docs-PR опубликованы);
осталось 5 конкретных правок «куда дотянулись» — они НЕ применены, дизайн ниже прибит.

---

## ЧАСТЬ 1. Апстрим — закрыто, ждём чужой реакции

- **[uglyoldbob/jni-min-helper#3](https://github.com/uglyoldbob/jni-min-helper/pull/3)
  СМЁРЖЕН** 07-25 00:35Z (`4a1bab4`), снят из draft, принят как есть. «I can make future
  refactoring based on this PR». ⇒ цель «разморозить PR» достигнута.
- **Замеры в [jni-rs#833](https://github.com/jni-rs/jni-rs/issues/833#issuecomment-5078164035)
  опубликованы** — репро UB на Linux/OpenJDK 21 + два эксперимента. Стенд:
  `/home/sc/t/jni833-repro/`.
- **Docs-PR [jni-rs#834](https://github.com/jni-rs/jni-rs/pull/834) открыт** — предусловие
  времён жизни загрузчика. Клон `/home/sc/t/jni-rs`, ветка
  `docs/native-method-registration-loader-lifetime`.
  ⚠ master jni-rs переехал в `crates/jni/src/` — плоские пути из 0.22.4 не работают.

**NEXT тут:** только ждать. Мейнтейнеров jni-rs в #833 не было ни до нас, ни на момент
публикации ⇒ ответ может быть небыстрым. Наш незакрытый долг по PR #3 — рантайм-тест
`PermissionRequestLifecycle` на MIUI-девайсе, нужен `cargo-apk2`.
Ещё не открыт **docs-PR в android-activity** («Runtime permissions» в README), #174 без
ответа мейнтейнера с 21.07 — двигается независимо от всего остального.

---

## ЧАСТЬ 2. Android-аудио — что ИЗМЕРЕНО на устройстве

Устройство: Redmi Note 10 Pro (`sweet`), Android 13, adb id `e10df2d7`.

### Пермишен — НЕ причина
`dumpsys package com.fretboard.snail` → `RECORD_AUDIO: granted=true` с 06-09
(`firstInstallTime`), флаги `USER_SET`. Фикс латчей `REQUESTED`/`KICKED` (коммит `847d316`)
закрывает РЕАЛЬНУЮ мину повторного `android_main`, но к сегодняшнему симптому отношения
не имеет — не выдавать его за починку звука.

### Подпись отказа на старой сборке (установка от 07-14)
Приложение рендерит (`frame N` растёт), разрешение выдано, и **ни одной строки AAudio за
всё время наблюдения** ⇒ поток захвата не открывался вообще.
(`FATAL EXCEPTION` × 135 в том же логе — это `com.miui.daemon` в крэш-цикле, MIUI-шум.)

### На свежей сборке HEAD — работает
```
AAudioStreamBuilder_openStream() returns 0 = AAUDIO_OK for s#1
capture started: id=cpal::aaudio:-1 rate=48000 monitor_out=0
[audioRecordData][fine] 55s(f:55001 m:103 s:0)
```
55 с непрерывного захвата; промахи `m:103` набежали в первые 5 с и дальше НЕ растут;
`s:0`; ноль `audio error`; кадры ~33 fps (совпадает с `request_repaint_after(33ms)`).
Скриншот телефона: спираль резонаторов горит живыми нотами. Два открытия потока —
штатно (старт + пересборка по фронту «выдано»).

### ⛔ Чего мы НЕ знаем: почему не работало
Установленный APK перезаписан `adb install`, а путь отказа не логировал ничего (дыра
закрыта в `847d316` — за час до того, как узнали про симптом). Post-mortem не осталось.
Разница между сборками: 9 коммитов аудио-работы 07-15/16 (rtswipe-фронтенд, ноль
аллокаций на аудио-потоке, real-FFT) + наша правка. Приписать одному чему-то нельзя.

**Как назвать причину, если решим её называть:** собрать `e56f7fd` (07-13 22:50 —
ближайший коммит до установки 07-14 04:56), поставить, подтвердить подпись «ноль AAudio»,
дальше половинным делением по 9 коммитам. 2–3 сборки по несколько минут каждая.
Решение «называть или забыть» — за юзером; чинить нечего, пока причина не названа.

---

## ЧАСТЬ 3. ПЛАН: 5 правок, дизайн прибит

Порядок — по убыванию ценности. Все решения уже приняты, переобсуждать не нужно.

### П1. Маршрут, которого не может быть, деградирует в дефолт (главное)
**Проблема:** `selected_input_id` персистится ([persist.rs:97](../../src/app/persist.rs#L97)),
а id вида `pulse::…` уводит захват в процесс `parec`, которого на Android нет. Отказ даёт
**ровно ту же подпись**, что мы видели: захват не открылся, AAudio молчит.
Сейчас причиной быть не может — `/data/data/com.fretboard.snail/files` ПУСТ, восстанавливать
нечего (eframe-стораж на Android не пишет). Механизм существует ⇒ закрыть по построению.

**Где:** `build_capture`, [mod.rs:708-716](../../src/audio/native/mod.rs#L708) — единственная
точка развилки pulse/cpal.

**Решения (прибиты):**
- Не «пробрасывать дальше»: `select_input_device` на незнакомый id отдаёт
  `Err("Input device not found: …")` ([devices.rs:272](../../src/audio/native/imp/devices.rs#L272)),
  т.е. молчаливый отказ сменился бы громким. Нужно **явно** превращать в `None`.
- `None` = вход по умолчанию ⇒ худшее последствие деградации — «не тот микрофон», не тишина.
- Все pulse-id имеют общий префикс `pulse::` (константы
  [mod.rs:124-126](../../src/audio/native/mod.rs#L124)) ⇒ хватает одной проверки префикса,
  `PULSE_DEFAULT_SOURCE_ID`/`_MONITOR_ID` покрыты ей же.
- Гейт — `cfg!(target_os = "linux")` (рантайм-if, не `#[cfg]`-блоки): компилится на всех
  платформах, не плодит мёртвых веток по таргетам. Pulse-путь = процесс `parec`, т.е.
  Linux-only по построению (это же написано в комментарии
  [devices.rs:32](../../src/audio/native/imp/devices.rs#L32)).
- Деградацию **логировать** через `audio_alog` — молчаливая подмена входа хуже отказа.

**DoD:** юнит-тест на хелпер (pulse-id → `None` вне Linux, сохраняется на Linux, cpal-id
не трогается никогда); `cargo check --target aarch64-linux-android --lib` + `--all-targets`.

### П2. Не плодить `parec`-спавны там, где Pulse быть не может
`pulse_input_available()` ([devices.rs:371](../../src/audio/native/imp/devices.rs#L371))
запускает процесс `parec` на КАЖДОМ перечислении устройств — на Android/macOS/Windows это
гарантированно провальный спавн. Ранний `false` вне Linux. Тот же `cfg!`, что в П1.
**DoD:** тесты парсера pulse-источников (они на строках, платформы не касаются) остаются зелёными.

### П3. Ошибки cpal-стримов не видны на Android
Четыре колбэка отдают ошибку через `eprintln!`:
[mod.rs:715](../../src/audio/native/mod.rs#L715) (дрон),
[1222](../../src/audio/native/mod.rs#L1222) (вход),
[1380](../../src/audio/native/mod.rs#L1380) (монитор),
[1433](../../src/audio/native/mod.rs#L1433) (тест-нота).
На устройстве `eprintln!` уходит в никуда. Ошибка приходит ИЗ колбэка драйвера, в `Result`
её не вернуть ⇒ лог — единственный канал. А отвал устройства на телефоне самый частый
случай (свернули приложение → AAudio закрыл вход).
**Решение:** хелпер `report_stream_error(what: &str, err: &cpal::StreamError)` = `audio_alog`
+ `eprintln!`; на не-Android `audio_alog` уже no-op, поведение десктопа не меняется.
**DoD:** все 4 колбэка через хелпер, ни одного `eprintln!` в аудио-модуле не осталось.

### П4. Закоммитить правку keystore в `Cargo.toml`
Висит незакоммиченной (`M Cargo.toml`, `M Cargo.lock`): путь заменён с
`~/.android/debug.keystore` на абсолютный, потому что cargo-apk **не раскрывает тильду** —
склеивает её с manifest dir и `apksigner` падает на FileNotFound. Без этой правки APK на
этой машине не подписывается, т.е. сборка держится на незакоммиченном изменении.
В `Cargo.lock` — только патч-бампы (`quick-xml` 0.39.4→0.41.0, `syn`, `wayland-backend`,
`foreign-types-macros`), к Android отношения не имеют; `cpal` остался **0.18.1** (проверено).

### П5. Release-профиль для APK
Телефонный APK — debug, а по коммиту `628a332` RT-SWIPE **в debug** съедает весь бюджет
кадра (16 мс = ровно период публикации `update_ms`) → поток публикации отстаёт → фриз;
в release тот же кадр 748 мкс (4.6% бюджета). Сейчас на телефоне выбран банк и всё ровно,
но переключение фронтенда на RT-SWIPE на debug-сборке даст фриз — и это будет НЕ новый баг.
Ключ для подписи release уже настроен (см. П4) ⇒ достаточно `cargo apk build --lib --release`.
**DoD:** release-APK ставится и стартует; переключение на RT-SWIPE не роняет кадры (мерить
по темпу `frame N` в логе, как в этой сессии).

---

## ЧАСТЬ 4. Рабочие команды (проверены сегодня)

```sh
# сборка + свежесть .so (грабли №1 — окаменелость от 04-26)
ANDROID_NDK_ROOT=/opt/android-sdk/android-ndk-r27c ANDROID_HOME=/opt/android-sdk \
  cargo apk build --lib
stat -c '%y %s %n' target/aarch64-linux-android/debug/libfretboard.so   # должно быть СЕГОДНЯ

adb install -r target/debug/apk/fretboard.apk
adb shell am start -n com.fretboard.snail/android.app.NativeActivity

# триаж: наш тег + системный аудио-слой; в файл, т.к. `| tail` при timeout теряет вывод
timeout 25 adb logcat -v time snail:V AAudio:V AAudioStream:V AudioRecord:V '*:S' > /tmp/log
adb exec-out screencap -p > /tmp/snail.png      # снейл живой?
adb shell dumpsys package com.fretboard.snail | grep RECORD_AUDIO
```

**Три развилки по логу:** нет `capture started`, есть `audio error: …` → вход не поднялся,
причина в строке · есть `capture started`, снейл молчит → вход открыт, сигнала нет ·
нет ни того, ни другого и `frame N` не растёт → до аудио не дошло (спящий экран).

**Грабли устройства:** MIUI по-прежнему блокирует `adb shell input keyevent`
(`SecurityException: INJECT_EVENTS`) ⇒ **экран будить физически**; без видимого окна eframe
не рендерит, аудио-путь не тронется. `dumpsys power | grep mWakefulness` — проверка.

# Kickoff — Android build «Resonator Snail» + runtime mic-permission

Хэндофф для продолжения в новой сессии. Дата: 2026-06-09.

## Цель
Собрать Android-версию `fretboard`. Приоритет — **resonators snail** (он и так весь
Android-UI: [`workspace.rs`](../../src/app/workspace.rs) `render()` под
`cfg(target_os="android")` рисует только `draw_resonator_snail_card`). Снейл
питается с микрофона → нужен рабочий рантайм-запрос `RECORD_AUDIO`.

## Что УЖЕ работает (проверено на устройстве)
- **Сборка/установка/запуск/рендер** — ок. APK: `target/debug/apk/fretboard.apk`.
- **Нативный аудио-стек компилится и работает под Android**: cpal **0.18.1** даёт
  AAudio-бэкенд (в логе `AAudioStreamBuilder ... dir = INPUT`), + rustfft/resonators/
  ringbuf. PulseAudio-пути (`parec`/`pactl`) на Android — рантайм-ноупы (фолбэк на
  дефолтный cpal-вход), компиляции не ломают.
- **Цикл eframe НЕ зависает** — добавлен «пульс» кадров, в logcat идут `frame 0,30,60…`.
  Ощущение «зависло» = статичное «waiting for input» (звука нет без разрешения).
- **При выданном вручную микрофоне снейл оживает**: `AAudio openStream() returns 0 =
  AAUDIO_OK`, банк резонаторов светится (скриншот подтверждён в той сессии).

## Окружение / устройство
- Тулинг: `cargo-apk 0.10.0`, NDK `r27c` в `/opt/android-sdk/android-ndk-r27c`,
  `ANDROID_HOME=/opt/android-sdk`, target `aarch64-linux-android`, adb, java 21.
- Команда сборки:
  ```sh
  ANDROID_NDK_ROOT=/opt/android-sdk/android-ndk-r27c ANDROID_HOME=/opt/android-sdk cargo apk build --lib
  ```
- Устройство: **Redmi Note 10 Pro** (`sweet`, M2101K6G), Android **13 / SDK 33**,
  arm64-v8a, adb id `e10df2d7`. Пакет: `com.fretboard.snail`, activity
  `android.app.NativeActivity`.

## ГЛАВНАЯ находка этой сессии (грабли №1)
`[lib] crate-type` был **`["rlib"]`**, а NativeActivity грузит **cdylib**
`libfretboard.so`. cargo собирал rlib, свежий `.so` НЕ производился, а cargo-apk молча
паковал **окаменелость от 2026-04-26** (`target/aarch64-linux-android/debug/libfretboard.so`,
259 МБ → 63 МБ в APK). Итог: **все правки Android-сборки месяцами не доезжали до
устройства**. Исправлено → `crate-type = ["rlib", "cdylib"]`.
- ✅ Проверка свежести: после сборки `stat target/aarch64-linux-android/debug/libfretboard.so`
  должен показывать СЕГОДНЯ. (Если что-то странное — `rm` этот .so и пересобрать.)

## Грабли №2 — логирование
Крейт `log` здесь **молчит** (какая-то зависимость вырезает его через
`log/max_level_*` фичу — даже `log::error!` не виден). Поэтому в
[`android_perm.rs`](../../src/android_perm.rs) сделан **прямой вывод** через
`__android_log_write` → `pub fn alog(&str)`, тег `snail`. Смотреть:
```sh
adb logcat -s snail
```
(`android_logger`/`log` из Cargo.toml убраны.)

## Грабли №3 — MIUI блокирует выдачу через adb
`pm grant ... RECORD_AUDIO` → `SecurityException (GRANT_RUNTIME_PERMISSIONS)`.
`appops set ... allow` ставит app-op, но runtime-флаг остаётся `granted=false`
(+ `Uid mode: ignore`). **Единственный рабочий ручной путь — Настройки →
Приложения → Resonator Snail → Разрешения → Микрофон → Разрешить.** Поэтому и нужен
честный in-app запрос.

## БЛОКЕР РЕШЁН (2026-06-09) ✅
Гипотеза подтверждена **по исходникам** android-activity 0.6.1 (не эмпирикой):
`init.rs:285` кладёт в `ndk_context` объект **Application** (через
`get_application(env, jni_activity)`), а НЕ Activity. Application — это `Context`,
поэтому `checkSelfPermission` (Context-метод) резолвился, а `requestPermissions`
(Activity-метод) — `MethodNotFound`.

**Фикс:** захват настоящего `NativeActivity` jobject через
`AndroidApp::activity_as_ptr()` (global ref на весь процесс, валиден до Drop
`AndroidApp`). `android_main` зовёт `android_perm::init_activity(ptr)`; новый
`static ACTIVITY: AtomicPtr` хранит его; `with_activity` ходит по нему (VM —
по-прежнему из `ndk_context`). UI-тред второго барьера НЕ возникло —
`requestPermissions` с потока рендера прошёл без исключения.

**Подтверждено на устройстве:** диалог показался → юзер нажал «Разрешить» →
`dumpsys package` показывает `RECORD_AUDIO: granted=true, flags=[USER_SET…]` →
re-kick пересобрал захват → снейл-спираль горит живыми нотами. Фолбэк на
настройки (план №4) НЕ понадобился.

⚠️ Грабли отладки: процесс на старте был `isSleeping=true/isVisible=false` (экран
спал) — eframe не рендерит без видимого окна, `render()` и запрос пермишена не
выполняются, пока экран не проснётся. MIUI блокирует `adb input keyevent`
(INJECT_EVENTS) → будить экран надо физически.

## Контриб в upstream (2026-07-21)

Разбор запощен в [rust-mobile/android-activity#174](https://github.com/rust-mobile/android-activity/issues/174#issuecomment-5035860576)
(аккаунт `enomado`): две **независимые** стены — (1) `ndk_context` = Application, не
Activity ⇒ `MethodNotFound` на `requestPermissions`, читается как опечатка в JNI-сигнатуре;
(2) колбэка `onRequestPermissionsResult` под NativeActivity нет вообще ⇒ только polling.
Issue открыт, без ассайни. В конце предложен docs-PR («Runtime permissions» в README) —
**NEXT = реакция мейнтейнера**; при тишине разумно открыть PR без приглашения.

Перепроверено 07-21: android-activity **0.6.1 всё ещё актуальна** (релиз 04.07.2026),
`get_application` → `initialize_android_context` на месте — разбор не устарел.

Крейт [`android-permissions`](https://crates.io/crates/android-permissions) 0.1.2 эту
дыру НЕ закрывает: требует `ndk_glue::native_activity()` (ndk_glue мёртв, вытеснен
android-activity), docs.rs его не собрал. Плодить свой крейт смысла нет — ценность в
знании, какой объект брать, а не в 170 строках.

### ⚠️ Известная мина: повторный `android_main()`
Доки `AndroidApp::activity_as_ptr()` явно предупреждают, что указатель **не `'static`**:
`android_main()` может запуститься повторно с новым `AndroidApp` (мы же в комментарии к
`ACTIVITY` писали «валиден на весь процесс» — это неточность). Сам указатель мы
переустанавливаем корректно (`init_activity` зовётся из `android_main` каждый раз), но
`REQUESTED`/`KICKED` в [`android_perm.rs`](../../src/android_perm.rs) — **процессные**
статики: на втором прогоне `android_main` диалог повторно не покажется. На устройстве
этого не ловили; фикс = сбрасывать оба флага в `init_activity`.

### (исторически) формулировка блокера до фикса
Рантайм-запрос исполнялся (`requesting RECORD_AUDIO via Activity.requestPermissions`),
но падал `JNI call failed: MethodNotFound { name: "requestPermissions", sig:
"([Ljava/lang/String;I)V" }`, хотя `checkSelfPermission` работал.

## Апстрим-сага mic-permission (полная хронология на 2026-07-25)

Три площадки, все связаны. Точка входа для новой сессии.

1. **[rust-mobile/android-activity#174](https://github.com/rust-mobile/android-activity/issues/174)** —
   тред-первоисточник. Наши 4 комментария (аккаунт `enomado`): разбор двух стен →
   уточнение после ответа автора (Стена 1 = **регрессия 0.6.1**, [PR #229](https://github.com/rust-mobile/android-activity/pull/229),
   раньше в ndk_context лежала Activity) → замер lifecycle → линк на PR.
2. **[enomado/android-permission-lifecycle-probe](https://github.com/enomado/android-permission-lifecycle-probe)** —
   публичный probe-репо. Замер на Redmi Note 10 Pro / A13 / MIUI: диалог даёт настоящий
   `Pause`/`Resume` (премиса дизайна ВЕРНА); уход-возврат = `Start` БЕЗ `Resume` (запрос
   честно висит pending). Локальная копия — `probes/lifecycle_probe/`.
3. **[uglyoldbob/jni-min-helper#3](https://github.com/uglyoldbob/jni-min-helper/pull/3)** —
   наш **draft PR**: dex-free `PermissionRequestLifecycle` через `ActivityLifecycleCallbacks`
   (register на Application, `requestPermissions` на Activity юзера, резолв на
   `onActivityResumed`). Компилится под aarch64, рантаймом в крейте НЕ тестирован. Форк
   `enomado/jni-min-helper`, ветка `lifecycle-permission-request`, клон `/home/sc/t/jni-min-helper`.

### ⛔ ГДЕ ЗАСТРЯЛИ (2026-07-24): soundness `jni`, НЕ наш код
Мейнтейнер (`wuwbobo2021`) PR не отклонил, но переключился на баг в самом `jni` и просит
помочь в диалоге с мейнтейнерами jni-rs:
[**jni-rs/jni-rs#833**](https://github.com/jni-rs/jni-rs/issues/833) (cc `@ColonelThirtyTwo`,
мейнтейнер jni-rs).

**Суть #833:** `jni` 0.22.4 продвигает *динамическую регистрацию* native-методов.
Класслоадер A (загрузил нативную библиотеку с обработчиками) ≠ класслоадер B класса C
(объявляет native-методы). GC уносит A → библиотека выгружается → живые инстансы C
остаются → вызов native-метода C = висячий указатель = **UB**. Воспроизводимый тест-кейс
на PC (URLClassLoader) есть в теле issue.

🔑 **КЛЮЧЕВОЙ НЮАНС для нас:** по его же анализу `android-activity`/Tauri **НЕ затронуты** —
все крейты собраны в ОДНУ `.so`, живущую до смерти процесса; выгрузка dylib на Android —
экзотика. Наш `DynamicProxy`/`InvocHdl` (PR #3) использует ту же механику native-методов,
но нашего сценария (NativeActivity, единая .so) баг **не касается**. То есть для НАШЕЙ цели
блокера нет; мейнтейнер держит паузу ради общей корректности крейта.

### NEXT SESSION (свежий бюджет — это тонкий разбор, наспех НЕ отвечать)
- Прочитать #833 целиком + референс [#526 comment](https://github.com/jni-rs/jni-rs/issues/526#issuecomment-5066800787),
  понять позицию мейнтейнеров jni-rs. Прогнать PC-репро (Java+cargo, есть в issue) —
  подтвердить UB своими глазами перед любым высказыванием.
- Полезный вклад в #833: (а) подтвердить/уточнить рассуждение «single sustained .so ⇒
  Android/NativeActivity вне зоны риска» — это снимает страх ИМЕННО для мобильного юзкейса;
  (б) обсудить, реалистичен ли `JNI_Unload`-хук для авто-разрегистрации (сам автор зовёт
  идею «probably bad»); (в) есть ли у jni-rs план документировать контракт вместо кода.
- Цель — разморозить PR #3. Если #833 уйдёт в долгую — можно предложить мейнтейнеру мёржить
  lifecycle-путь с оговоркой в доке (для Android он sound), не дожидаясь общего решения.
- Параллельно всё ещё открыт **docs-PR в android-activity** (секция «Runtime permissions»),
  его можно двигать независимо.

## ПЛАН на следующую сессию
1. **Подтвердить тип объекта** (в `request_record_audio`, разово через `alog`):
   - `env.is_instance_of(activity, jni_str!("android/app/Activity"))?` → bool;
   - `env.is_instance_of(activity, jni_str!("android/app/NativeActivity"))?`;
   - при желании имя класса: `getClass().getName()` → `env.get_string(&jstr)?.to_string()`
     (`get_string` возвращает `MUTF8Chars`, у него есть `to_string()`).
2. **Если это не Activity — достать NativeActivity jobject:**
   - В `android_main` есть `android_app: AndroidApp`. Для native-activity у
     `ANativeActivity` поле `.clazz` = Java-объект NativeActivity. Достать его
     (через `ndk`/`android-activity` API или `android_app.activity_as_ptr()` →
     `*const ANativeActivity` → `(*p).clazz`), создать **global ref**, положить в
     `static` в `android_perm` и вызывать `requestPermissions` на нём.
   - Альтернатива: проверить, нет ли в android-activity 0.6 прямого доступа к
     activity jobject.
3. **Вызвать `requestPermissions` на Activity.** Возможен второй барьер —
   `requestPermissions` может требовать **UI-поток** (мы на потоке рендера). Сначала
   просто попробовать; если кинет — постить на UI-тред (сложно без Java Runnable).
4. **Если popup на MIUI останется капризным — фолбэк:** через JNI
   `Context.startActivity(Intent(ACTION_APPLICATION_DETAILS_SETTINGS, package:...))`
   (`startActivity` — метод **Context**, доступен на том, что уже есть) → юзер выдаёт
   микрофон в настройках. UX замыкается существующим re-kick (см. ниже).
5. **Re-kick уже готов и проверен по механике:** [`workspace.rs`](../../src/app/workspace.rs)
   android-`render()` зовёт `request_record_audio()` (fire-once) и
   `newly_granted()`; по фронту «выдано» делает `self.audio.set_selected_input_id(None)`
   → `AudioEngine` пересобирает захват через `SwitchInput` (стартовый open до выдачи
   падает — это норм). Ручная выдача в прошлой сессии так и оживила снейл.

## Состояние кода (всё закоммичено в рабочее дерево, не в git)
- `Cargo.toml`: `crate-type=["rlib","cdylib"]`; `[[…android.uses_permission]]
  RECORD_AUDIO`; activity `orientation="sensorLandscape"` (ключ именно `orientation`,
  не `screen_orientation` — иначе теряется); deps `jni="0.22"`, `ndk-context="0.1"`
  (оба уже в дереве транзитивно).
- `src/android_perm.rs`: `alog`, `with_activity` (closure-attach под **новый jni
  0.22.4 API**: `EnvUnowned`/`Env`, `attach_current_thread(|env| …)`,
  `jni_str!`/`jni_sig!`), `record_audio_granted`, `request_record_audio`,
  `newly_granted`.
- `src/lib.rs`: `#[cfg(android)] pub mod android_perm;`, `android_main` зовёт
  `alog("android_main start")`.
- `src/app/workspace.rs`: heartbeat (можно убрать после отладки) + permission-driver
  в android-`render()`.

## Полезные API-заметки (jni 0.22.4 — сильно переработан против 0.21)
- `JavaVM::from_raw(ptr) -> Self` (не Result; идемпотентен — singleton).
- `JObject::from_raw(&env, raw) -> JObject` (2 аргумента!).
- `vm.attach_current_thread(|env: &mut Env| -> Result<T,E> { … })` (closure, permanent).
- `call_method(obj, jni_str!("name"), jni_sig!("(…)…"), &[JValue::…])`; имя =
  `AsRef<JNIStr>` → `jni_str!`; сигнатура = `AsRef<MethodSignature>` → `jni_sig!`
  (принимает СЫРУЮ JNI-строку, напр. `jni_sig!("(I)I")`).
- `JValue::Object(&jstring)` ок (deref-coercion JString/JObjectArray → JObject);
  `JValue::Int(0)`; `.i()`/`.l()`/`.z()` на результате.
- `new_object_array(len, jni_str!("java/lang/String"), &perm)` (element_class =
  `Desc<JClass>`, годится `jni_str!`).
- `is_instance_of(obj, class: Desc<JClass>)`.

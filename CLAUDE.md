## Coding Style

### Error Handling
- Use `unwrap()` instead of `?` operator or `match` where possible.
  - Инварианты проверяются до попадания в аргументы → чище код
  - API и БД-квери → исключение (нужен Result)
  - Критичное место падает? → решай пользователь, можно Result

### Type Safety
- Prefer newtypes over raw indices wherever possible
- Instead of `usize`, define dedicated index types, e.g.:
```rust
  struct ProviderId(pub usize);
  struct EdgeId(usize);
```
- Use newtypes to prevent mixing up different index domains at compile time
- Avoid `impl Deref`. prefer `newtype.0`

### Avoid Option as a Crutch
- Не использовать `Option` как затычку «пока не знаю значение» — это прячет баги.
- Дефолтные значения (0, пустая строка) — ещё хуже: тихо маскируют отсутствие данных.
- Предпочитать `unwrap()` — fail-fast, контрактное программирование: если значение обязано быть, пусть падает.
- `Option` допустим только когда отсутствие значения — семантически значимо и неизбежно.

### Comments
- Писать комментарии щедро: инварианты, неочевидные свойства, сложные алгоритмы
- Если есть контракт/предусловие — задокументировать в комментарии
- Сложная логика (нетривиальные формулы, хитрые индексы, неочевидный порядок) — объяснять «почему», а не «что»
- Не стесняться многострочных комментариев, если это помогает понять код

### Re-exports
- Avoid re-exports (`pub use`). Import types directly from their source modules instead.
  - Re-exports obscure where types actually live
  - They create hidden coupling between modules
  - Prefer explicit paths at call sites

### Code Reuse
- **Не дублировать код** — если алгоритм уже есть, использовать и расширять его.
- Перед написанием нового — искать существующие методы/функции в кодовой базе.
- Реюз кода = гарантия корректности: существующий код проверен и работает, новый — нет.
- Расширяй существующие структуры и функции, а не копируй с вариацией.

### Tools
- target directory is overriden: /tmp/rust_target. sometimes needs to do cargo clean when full

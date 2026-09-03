//! Выход ансамбля: веса методов, их оценки на одном кандидате и перевод оценок
//! в вероятности. Сам расчёт методов живёт в `method_*`, здесь — только то, чем
//! они складываются в один вердикт.

/// Веса ансамбля методов. По умолчанию набор/профиль ведут, корень/спираль
/// уточняют тонику (A — набор нот, B — мажор/минор + гравитация, C — бас+устойчивость,
/// D — центр тяжести на круге квинт).
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct MethodWeights {
    pub set:     f32, // метод A — косинус с плоской маской
    pub profile: f32, // метод B — Пирсон с тональным профилем
    pub root:    f32, // метод C — улика корня (бас + устойчивость)
    #[serde(default)]
    pub spiral:  f32, // метод D — центр тяжести на круге квинт
}

impl Default for MethodWeights {
    fn default() -> Self {
        Self {
            set:     0.3,
            profile: 0.3,
            root:    0.2,
            spiral:  0.2,
        }
    }
}

/// Оценки методов для одного кандидата, каждая нормирована в [0, 1].
#[derive(Clone, Copy)]
pub struct MethodScores {
    pub set:     f32,
    pub profile: f32,
    pub root:    f32,
    pub spiral:  f32,
}

impl MethodScores {
    /// Взвешенное среднее методов — итоговая оценка кандидата в [0, 1].
    pub fn blended(&self, weights: MethodWeights) -> f32 {
        let total = (weights.set + weights.profile + weights.root + weights.spiral).max(1e-6);
        (weights.set * self.set
            + weights.profile * self.profile
            + weights.root * self.root
            + weights.spiral * self.spiral)
            / total
    }
}

/// Конфиг панели Scale Finder: баланс методов + ширина окна интеграции В СЕКУНДАХ
/// (решалка копит свой буфер по времени, не привязана к длине истории банка).
/// Узкое окно отзывчиво, но дёргано; широкое стабильно, но инертно.
#[derive(Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct ScaleFinderConfig {
    pub weights:        MethodWeights,
    #[serde(default = "default_window_seconds")]
    pub window_seconds: f32,
}

fn default_window_seconds() -> f32 {
    4.0
}

impl Default for ScaleFinderConfig {
    fn default() -> Self {
        Self {
            weights:        MethodWeights::default(),
            window_seconds: default_window_seconds(),
        }
    }
}

/// Softmax с температурой: переводит близко лежащие оценки кандидатов в
/// распределение вероятностей. Меньшая `temperature` — острее пик на лидере.
pub fn softmax_with_temperature(scores: &[f32], temperature: f32) -> Vec<f32> {
    if scores.is_empty() {
        return Vec::new();
    }
    let t = temperature.max(1e-4);
    let max = scores.iter().copied().fold(f32::MIN, f32::max);
    let exps: Vec<f32> = scores.iter().map(|s| ((s - max) / t).exp()).collect();
    let sum: f32 = exps.iter().sum();
    if sum <= 0.0 {
        return vec![0.0; scores.len()];
    }
    let inv = 1.0 / sum;
    exps.iter().map(|e| e * inv).collect()
}

#[cfg(test)]
mod tests {
    use super::softmax_with_temperature;

    #[test]
    fn softmax_sums_to_one_and_peaks_on_max() {
        let probs = softmax_with_temperature(&[0.9, 0.6, 0.6, 0.3], 0.06);
        let sum: f32 = probs.iter().sum();
        assert!((sum - 1.0).abs() < 1e-4);
        assert!(probs[0] > probs[1]);
    }
}

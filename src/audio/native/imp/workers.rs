//! Analysis workers: the threads that drain the capture rings into the pipelines.
//!
//! One loop for both planes ([`start_worker`]); the only fork between them is whether
//! the worker may park — see [`WorkerPipeline`].

use std::sync::atomic::{
    AtomicBool,
    AtomicU32,
    Ordering,
};
use std::sync::{
    Arc,
    Mutex,
};
use std::thread::{
    self,
    JoinHandle,
};
use std::time::{
    Duration,
    Instant,
};

use ringbuf::HeapRb;
use ringbuf::traits::{
    Consumer,
    Split,
};

use super::{
    ANALYSIS_IDLE_SLEEP,
    SampleConsumer,
    SampleProducer,
};
use crate::audio::core::{
    AnalysisPipeline,
    ResonatorPipeline,
    SharedState,
};
use crate::audio::sample_rate::SampleRate;
use crate::audio::types::AnalysisSettings;

/// Сон запаркованного резонаторного воркера между сливами кольца.
const RESONATOR_PARK_SLEEP: Duration = Duration::from_millis(20);

pub(super) struct AnalysisWorker {
    pub(super) stop:   Arc<AtomicBool>,
    pub(super) thread: JoinHandle<()>,
}

impl AnalysisWorker {
    pub(super) fn stop(self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.thread.join();
    }
}

/// Что считает воркер. Пайплайнов ровно два, и различаются они для петли одной
/// развилкой — паркуется ли воркер, — поэтому `enum`, а не трейт.
#[expect(
    clippy::large_enum_variant,
    reason = "один экземпляр на поток воркера, переезжает в него один раз — размер варианта ничего не стоит"
)]
pub(super) enum WorkerPipeline {
    Analysis(AnalysisPipeline),
    /// Считает, только пока UI сдвигает дедлайн вперёд; иначе кольцо дренируется вхолостую.
    Resonator {
        pipeline: ResonatorPipeline,
        wanted:   Arc<Mutex<Instant>>,
    },
}

impl WorkerPipeline {
    /// Запаркован ли воркер: анализ — никогда, банк — когда дедлайн UI в прошлом
    /// (панель-потребитель закрылась, см. [`super::RESONATOR_PARK_GRACE`]).
    fn parked(&self) -> bool {
        match self {
            Self::Analysis(_) => false,
            Self::Resonator { wanted, .. } => Instant::now() >= *wanted.lock().unwrap(),
        }
    }

    fn push_samples(
        &mut self,
        samples: impl IntoIterator<Item = f32>,
        shared: &Arc<Mutex<SharedState>>,
        settings: &Arc<Mutex<AnalysisSettings>>,
        input_gain: &Arc<AtomicU32>,
        input_level: &Arc<AtomicU32>,
    ) {
        match self {
            Self::Analysis(pipeline) => {
                pipeline.push_samples(samples, shared, settings, input_gain, input_level)
            }
            Self::Resonator { pipeline, .. } => {
                pipeline.push_samples(samples, shared, settings, input_gain, input_level)
            }
        }
    }
}

/// Дренируем сколько есть в кольце, не больше 4096 за раз, чтобы FFT-пауза не
/// превышала одного сэмпл-окна.
fn pop_batch(cons: &mut SampleConsumer, batch: &mut Vec<f32>) {
    for _ in 0..4096 {
        match cons.try_pop() {
            Some(s) => batch.push(s),
            None => break,
        }
    }
}

/// `input_level` пишет анализ-воркер, а банк читает: это гейт тишины мелодической
/// линии. Колонка банка нормирована и тишину от шума отличить не может — абсолютный
/// уровень меряется на другой плоскости и передаётся сюда.
pub(super) fn start_worker(
    mut cons: SampleConsumer,
    mut pipeline: WorkerPipeline,
    shared: Arc<Mutex<SharedState>>,
    settings: Arc<Mutex<AnalysisSettings>>,
    input_gain: Arc<AtomicU32>,
    input_level: Arc<AtomicU32>,
) -> AnalysisWorker {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = stop.clone();

    let thread = thread::spawn(move || {
        let mut batch: Vec<f32> = Vec::with_capacity(4096);

        while !stop_flag.load(Ordering::Relaxed) {
            batch.clear();
            pop_batch(&mut cons, &mut batch);
            // Дедлайн смотрим ПОСЛЕ вычерпывания: запаркованный воркер кольцо ВСЁ
            // РАВНО дренирует (иначе оно переполнится и при пробуждении выльется
            // пачкой старого звука), но дорогой банк не считает — это и есть
            // экономия CPU.
            if pipeline.parked() {
                thread::sleep(RESONATOR_PARK_SLEEP);
                continue;
            }
            if batch.is_empty() {
                thread::sleep(ANALYSIS_IDLE_SLEEP);
                continue;
            }
            pipeline.push_samples(batch.drain(..), &shared, &settings, &input_gain, &input_level);
        }
    });

    AnalysisWorker { stop, thread }
}

// Кольцевой буфер для анализа. Размер — 0.5с при данном rate,
// с большим запасом на подёргивания планировщика.
pub(super) fn analysis_ring(sample_rate: SampleRate) -> (SampleProducer, SampleConsumer) {
    HeapRb::<f32>::new(sample_rate.samples_in(Duration::from_millis(500))).split()
}

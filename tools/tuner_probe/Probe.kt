import de.moekadu.tuner.notedetection.*
import kotlinx.coroutines.runBlocking
import java.io.File
import javax.sound.sampled.AudioFormat
import javax.sound.sampled.AudioSystem

// The application's collector and evaluator are compiled unchanged. Only file input,
// Android logging/resources and CSV output are supplied by this command-line adapter.
fun main(args: Array<String>) = runBlocking {
    val output = File(args[0])
    val pool = MemoryPoolSampleData(4)
    for (path in File(args[1]).readLines().filter { it.isNotBlank() }) {
        val wav = File(path)
        val stream = AudioSystem.getAudioInputStream(wav)
        val format = stream.format
        require(format.encoding == AudioFormat.Encoding.PCM_SIGNED && format.sampleSizeInBits == 16 && !format.isBigEndian)
        val rate = format.sampleRate.toInt()
        val bytes = stream.use { it.readBytes() }
        val samples = FloatArray(bytes.size / format.frameSize) { frame ->
            var sum = 0f
            for (channel in 0 until format.channels) {
                val offset = frame * format.frameSize + 2 * channel
                val sample = ((bytes[offset].toInt() and 255) or (bytes[offset + 1].toInt() shl 8)).toShort()
                sum += sample.toFloat() / 32768f
            }
            sum / format.channels
        }
        // Defaults from SimpleFrequencyDetector and ReferenceResources at the pinned commit.
        val collector = FrequencyDetectionResultCollector(16f, 16000f, 0.1f, 0.75f,
            0.11f, 3f, 5, 8, WindowingFunction.Tophat, AcousticZeroWeighting())
        val evaluator = FrequencyEvaluatorSimple(5, 3, 0.1f, 0.1f, 90f)
        val writers = listOf("raw", "line").map { stage ->
            File(output, "$stage/${wav.nameWithoutExtension}.csv").apply { parentFile.mkdirs() }.bufferedWriter()
        }
        var position = 0
        while (position + 4096 <= samples.size) {
            val data = pool.get(4096, rate, position)
            samples.copyInto(data.memory.data, 0, position, position + 4096)
            val result = collector.collectResults(data)
            val raw = result.memory.frequency
            val line = evaluator.evaluate(result.memory)
            // Right edge: when the complete window is available, without backdating.
            val time = (position + 4096).toDouble() / rate
            require(raw.isFinite() && line.isFinite())
            writers[0].appendLine("$time,$raw")
            writers[1].appendLine("$time,$line")
            result.decRef()
            data.decRef()
            position += 1024 // default 75% overlap
        }
        writers.forEach { it.close() }
        println(wav.name)
    }
}

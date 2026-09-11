import me.iverins.csc.util.DeterministicRandom

/**
 * 生成 csc-util RNG 的跨语言 golden 序列（Kotlin 侧权威输出）。
 *
 * 运行（在 kotlin_prototype 目录）：
 *   kotlinc src/me/iverins/csc/util/DeterministicRandom.kt tools/gen_rng_golden.kt -include-runtime -d /tmp/golden.jar
 *   java -jar /tmp/golden.jar
 *
 * 输出固化到 backend/crates/csc-util/src/rng.rs 的 GOLDEN_* 常量。
 * 所有值以「位模式」输出（f64/f32 用 toRawBits），避免跨语言十进制打印差异。
 */
fun main() {
    fun seqLong(seed: Long, n: Int): String =
        (0 until n).joinToString(",") {
            val r = DeterministicRandom.seed(seed)
            repeat(it) { r.nextLong() }
            "%016x".format(r.nextLong())
        }

    fun seqDoubleBits(seed: Long, n: Int): String =
        (0 until n).joinToString(",") {
            val r = DeterministicRandom.seed(seed)
            repeat(it) { r.nextDouble() }
            "%016x".format(r.nextDouble().toRawBits())
        }

    fun seqFloatBits(seed: Long, n: Int): String =
        (0 until n).joinToString(",") {
            val r = DeterministicRandom.seed(seed)
            repeat(it) { r.nextFloat() }
            "%08x".format(r.nextFloat().toRawBits())
        }

    fun seqInt(seed: Long, until: Int, n: Int): String =
        (0 until n).joinToString(",") {
            val r = DeterministicRandom.seed(seed)
            repeat(it) { r.nextInt(until) }
            r.nextInt(until).toString()
        }

    // 核心序列：连续调用（共享实例）
    fun seqLongShared(seed: Long, n: Int): String {
        val r = DeterministicRandom.seed(seed)
        return (0 until n).joinToString(",") { "%016x".format(r.nextLong()) }
    }
    fun seqDoubleShared(seed: Long, n: Int): String {
        val r = DeterministicRandom.seed(seed)
        return (0 until n).joinToString(",") { "%016x".format(r.nextDouble().toRawBits()) }
    }
    fun seqFloatShared(seed: Long, n: Int): String {
        val r = DeterministicRandom.seed(seed)
        return (0 until n).joinToString(",") { "%08x".format(r.nextFloat().toRawBits()) }
    }
    fun seqIntShared(seed: Long, until: Int, n: Int): String {
        val r = DeterministicRandom.seed(seed)
        return (0 until n).joinToString(",") { r.nextInt(until).toString() }
    }

    println("LONG42=" + seqLongShared(42, 10))
    println("DOUBLE42=" + seqDoubleShared(42, 5))
    println("FLOAT42=" + seqFloatShared(42, 3))
    println("INT100_42=" + seqIntShared(42, 100, 10))
    println("INT97_42=" + seqIntShared(42, 97, 10))
    println("INT64_42=" + seqIntShared(42, 64, 8))
    println("INT1_42=" + seqIntShared(42, 1, 4))
    println("LONG7=" + seqLongShared(7, 5))
    println("LONG0=" + seqLongShared(0, 3))

    // 快照/恢复对照：取 2 → 快照 → 取 3 → 恢复 → 取 3（应等于前 3）
    val r = DeterministicRandom.seed(42)
    r.nextLong(); r.nextLong()
    val snap = r.snapshot()
    val after = (0 until 3).joinToString(",") { "%016x".format(r.nextLong()) }
    val restored = DeterministicRandom.restore(snap)
    val replay = (0 until 3).joinToString(",") { "%016x".format(restored.nextLong()) }
    println("SNAPSHOT_AFTER=$after")
    println("SNAPSHOT_REPLAY=$replay")
    println("SNAPSHOT_STATE=" + snap.joinToString(",") { "%016x".format(it) })
}

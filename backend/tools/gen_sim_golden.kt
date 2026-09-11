import me.iverins.csc.domain.TourneyTier
import me.iverins.csc.entities.BaseAttributes
import me.iverins.csc.entities.NPC
import me.iverins.csc.entities.ProAttributes
import me.iverins.csc.entities.Role
import me.iverins.csc.entities.SkillAttributes
import me.iverins.csc.entities.Team
import me.iverins.csc.entities.WeaponAttributes
import me.iverins.csc.hltv.RatingCalculator
import me.iverins.csc.simulation.MatchSimulator
import me.iverins.csc.simulation.WinRateCalculator
import me.iverins.csc.util.DeterministicRandom

/**
 * 生成 csc-simulation 的跨语言 golden 序列（Kotlin 侧权威输出）。
 * 运行：kotlinc $(find src -name "*.kt") tools/gen_sim_golden.kt -include-runtime -d /tmp/golden3.jar
 * 输出固化到 backend/crates/csc-simulation/src/ 的 golden 测试。
 */
fun main() {
    fun bits(v: Double): String = "%016x".format(v.toRawBits())

    // 固定属性队伍：A 队 5×RIFLER 全 80；B 队 5×RIFLER 全 70
    // 构造顺序：先占位 team（空 roster）建 NPC，再建带 roster 的正式 team（签名含选手名）
    fun squad(prefix: String, power: Int): Pair<Team, List<NPC>> {
        val placeholder = Team(prefix, 1, 100, emptyList())
        val npcs = (0 until 5).map { i ->
            NPC(
                playerName = "$prefix$i", age = 24, role = Role.RIFLER,
                base = BaseAttributes(power, power, power, power, power),
                skill = SkillAttributes(power, power, power, power),
                pro = ProAttributes(power, power, power, power, power),
                weapon = WeaponAttributes(Role.RIFLER, power, power, power, power, power),
                team = placeholder, potential = 100
            )
        }
        return Team(prefix, 1, 100, npcs) to npcs
    }
    val (teamA, rosterA) = squad("A", 80)
    val (teamB, rosterB) = squad("B", 70)

    // 1. winRate：seed=42 连续 5 次（位模式）
    val r1 = DeterministicRandom.seed(42)
    println("WINRATE_80_70=" + (0 until 5).joinToString(",") {
        WinRateCalculator.winRate(rosterA, rosterB, TourneyTier.T1, r1).toRawBits().let { v -> "%016x".format(v) }
    })

    // 2. overtimeChance(0.5)（位模式）
    println("OT_0_5=" + bits(MatchSimulator.overtimeChance(0.5)))

    // 3. simulateMap：seed=42 一张图（比分/胜者/首行战绩）
    val r2 = DeterministicRandom.seed(42)
    val map = MatchSimulator.simulateMap(1, TourneyTier.T1, teamA, teamB, r2)
    println("MAP_SCORE=${map.teamAScore},${map.teamBScore}")
    println("MAP_WINNER=${map.winnerSig}")
    val l0 = map.lines.first()
    println("MAP_LINE0=${l0.playerName},${l0.kills},${l0.deaths},${l0.assists}")

    // 4. ratingOf counts
    println("RATING_COUNTS=" + bits(RatingCalculator.ratingOf(20, 15, 5, 30)))
    println("ADR=" + bits(RatingCalculator.adrOf(0.8, 0.25, 0.6)))
    println("KAST=" + bits(RatingCalculator.kastOf(0.8, 0.6, 0.25)))
}

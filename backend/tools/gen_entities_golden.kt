import me.iverins.csc.domain.Tier
import me.iverins.csc.entities.Attr
import me.iverins.csc.entities.PowerCalculator
import me.iverins.csc.entities.RandomPlayerGenerator
import me.iverins.csc.entities.RoleProfiles
import me.iverins.csc.entities.Role
import me.iverins.csc.util.DeterministicRandom

/**
 * 生成 csc-entities 的跨语言 golden 序列（Kotlin 侧权威输出）。
 *
 * 运行（在 kotlin_prototype 目录）：
 *   kotlinc $(find src -name "*.kt") tools/gen_entities_golden.kt -include-runtime -d /tmp/golden2.jar
 *   java -jar /tmp/golden2.jar
 *
 * 输出固化到 backend/crates/csc-entities/src/ 的 golden 测试。
 * f64 一律以 toRawBits 位模式输出，避免跨语言十进制打印差异。
 */
fun main() {
    // 1. statValue：固定输入
    val stat = RoleProfiles.statValue(80..98, 0.85, 0.5, 0.3)
    println("STAT1=$stat")
    println("STAT2=" + RoleProfiles.statValue(0..100, 0.5, 0.5, 0.5))

    // 2. playerPower：固定属性组 × 3 角色（位模式）
    fun bits(v: Double): String = "%016x".format(v.toRawBits())
    val base = me.iverins.csc.entities.BaseAttributes(80, 80, 80, 80, 80)
    val skill = me.iverins.csc.entities.SkillAttributes(80, 50, 50, 70)
    val pro = me.iverins.csc.entities.ProAttributes(70, 70, 60, 50, 60)
    val weapon = me.iverins.csc.entities.WeaponAttributes(Role.AWP, 60, 90, 80, 50, 50)
    println("POWER_IGL=" + bits(PowerCalculator.playerPower(Role.IGL, base, skill, pro, weapon)))
    println("POWER_AWP=" + bits(PowerCalculator.playerPower(Role.AWP, base, skill, pro, weapon)))
    println("POWER_RIFLER=" + bits(PowerCalculator.playerPower(Role.RIFLER, base, skill, pro, weapon)))

    // 3. generateAttributes：固定种子/档位/角色/名字/年龄（全属性 + 潜力）
    val rng = DeterministicRandom.seed(42)
    val g = RandomPlayerGenerator.generateAttributes(Tier.TIER2, Role.RIFLER, "Golden", rng, 20)
    println("GEN_ROLE=" + g.role)
    println("GEN_AGE=$age")
    println("GEN_POTENTIAL=${g.potential}")
    println("GEN_BASE=${g.base.reaction},${g.base.stability},${g.base.endurance},${g.base.stamina},${g.base.health}")
    println("GEN_SKILL=${g.skill.aim},${g.skill.leader},${g.skill.communication},${g.skill.clutch}")
    println("GEN_PRO=${g.pro.mentality},${g.pro.confidence},${g.pro.teamSpirit},${g.pro.loyalty},${g.pro.morale}")
    println("GEN_WEAPON=${g.weapon.position},${g.weapon.ak},${g.weapon.awp},${g.weapon.pistol},${g.weapon.smoke},${g.weapon.utility}")

    // 4. potentialFor 序列（固定种子连续 3 次，age=20）
    val rng2 = DeterministicRandom.seed(42)
    val pot = (0 until 3).joinToString(",") { rng2.nextLong(); RandomPlayerGenerator.potentialFor(20, 60, rng2).toString() }
    println("POT_20_60=$pot")

    // 5. Attr 偏好读取（验证数组顺序与 Kotlin map 一致）：AWP 画像的 awp/ak pref
    val awpProfile = RoleProfiles.of(Role.AWP)
    println("PREF_AWP_AWP=" + awpProfile.pref(Attr.AWP))
    println("PREF_AWP_AK=" + awpProfile.pref(Attr.AK))
    println("VOL_AWP=${awpProfile.volatility}")
}

val age = 20

// shared_defaults.mjs — 默认决策唯一前端镜像（Node 侧，供 smoke 与 scripts/ 驱动脚本共享）。
//
// 三方对齐契约：
//   本文件 ↔ 后端 csc-decision/src/source.rs::default_decision_for（唯一事实源）。
//   csc-decision/src/source.rs::default_decision_for（唯一事实源）。
// 改动必须同步后端并跑 C-E1 golden 测试与本文件 selfcheck()。
//
// 来源：decide.ts:57-107 defaultDecision（C-E1 修正后语义，含 TEAMMATE_INVITE 强制
// DECLINE）。Node ≥18 ESM，无外部依赖，CI `node --check` 门禁兼容。

/**
 * 单个决策点的默认决策（与后端 AutoDecisionSource::default_decision_for 语义对齐）。
 * @param {object} p 决策点（externally tagged：{ KIND: data }）
 * @returns {{ point_id: string, option_id: string }}
 */
export function defaultDecision(p) {
  const [kind, d] = Object.entries(p)[0];
  switch (kind) {
    case "TRAINING_FOCUS":
      return { point_id: d.id, option_id: d.options[0]?.focus ?? "AIM" };
    case "TRANSFER_WINDOW": {
      // 只跳槽到明显更强的队伍（候选排名 < 当前队排名），否则续约留队。
      const best = d.offers[0]?.candidates[0];
      const current = d.offers[0]?.current_ranking;
      const shouldJump = best && (current == null || best.ranking < current);
      return { point_id: d.id, option_id: shouldJump ? best.team_signature : "STAY" };
    }
    case "MATCH_INTERVENTION":
      return { point_id: d.id, option_id: d.options[0]?.id ?? "DEFAULT" };
    case "TEAMMATE_BLUNDER":
      return { point_id: d.id, option_id: "IGNORE" };
    case "INJURY_DECISION":
      // 与后端一致：默认带伤也打（职业态度，累积伤病风险）。
      return { point_id: d.id, option_id: "PLAY_THROUGH" };
    case "SPONSORSHIP_OFFER":
      return { point_id: d.id, option_id: "ACCEPT" };
    case "LIFE_EVENT": {
      // 与后端一致：假赛联系必拒绝、宫斗必调停、队友邀请不静默承接（DECLINE）。
      if (d.kind === "MATCH_FIXER_CONTACT") return { point_id: d.id, option_id: "REFUSE" };
      if (d.kind === "TEAMMATE_POWER_STRUGGLE") return { point_id: d.id, option_id: "MEDIATE" };
      if (d.kind === "TEAMMATE_INVITE") return { point_id: d.id, option_id: "DECLINE" };
      return { point_id: d.id, option_id: d.options[0]?.id ?? "DECLINE" };
    }
    default:
      throw new Error(`未知决策点类型：${kind}`);
  }
}

// —— 自检 golden 表（与 C-E1 decide.test.ts 同款 13 场景，smoke 启动时跑） ——

const GOLDEN = [
  {
    name: "TRANSFER 候选更强 → 跳槽",
    point: { TRANSFER_WINDOW: { id: "t1", offers: [{ current_ranking: 12, candidates: [{ team_signature: "NEW", ranking: 5 }] }] } },
    expect: "NEW",
  },
  {
    name: "TRANSFER 候选更弱 → 留队",
    point: { TRANSFER_WINDOW: { id: "t2", offers: [{ current_ranking: 5, candidates: [{ team_signature: "NEW", ranking: 12 }] }] } },
    expect: "STAY",
  },
  {
    name: "TRANSFER 自由身 → 跳槽",
    point: { TRANSFER_WINDOW: { id: "t3", offers: [{ current_ranking: null, candidates: [{ team_signature: "NEW", ranking: 20 }] }] } },
    expect: "NEW",
  },
  {
    name: "TRANSFER 无候选 → 留队",
    point: { TRANSFER_WINDOW: { id: "t4", offers: [{ current_ranking: 12, candidates: [] }] } },
    expect: "STAY",
  },
  {
    name: "TRAINING 取首项/空 → AIM",
    point: { TRAINING_FOCUS: { id: "tf1", options: [{ focus: "UTILITY" }] } },
    expect: "UTILITY",
  },
  {
    name: "TRAINING 空选项 → AIM",
    point: { TRAINING_FOCUS: { id: "tf2", options: [] } },
    expect: "AIM",
  },
  {
    name: "MATCH 取首项/空 → DEFAULT",
    point: { MATCH_INTERVENTION: { id: "mi1", options: [{ id: "AGGRESSIVE" }] } },
    expect: "AGGRESSIVE",
  },
  {
    name: "BLUNDER → IGNORE",
    point: { TEAMMATE_BLUNDER: { id: "b1" } },
    expect: "IGNORE",
  },
  {
    name: "INJURY → PLAY_THROUGH（非 options[0]）",
    point: { INJURY_DECISION: { id: "i1", options: [{ id: "REST" }] } },
    expect: "PLAY_THROUGH",
  },
  {
    name: "SPONSOR → ACCEPT",
    point: { SPONSORSHIP_OFFER: { id: "s1", options: [] } },
    expect: "ACCEPT",
  },
  {
    name: "LIFE 假赛 → REFUSE",
    point: { LIFE_EVENT: { id: "l1", kind: "MATCH_FIXER_CONTACT", options: [{ id: "TAKE" }] } },
    expect: "REFUSE",
  },
  {
    name: "LIFE 宫斗 → MEDIATE",
    point: { LIFE_EVENT: { id: "l2", kind: "TEAMMATE_POWER_STRUGGLE", options: [{ id: "BACK_ACTOR" }] } },
    expect: "MEDIATE",
  },
  {
    name: "LIFE 队友邀请 → DECLINE（非 options[0]=JOIN）",
    point: { LIFE_EVENT: { id: "l3", kind: "TEAMMATE_INVITE", options: [{ id: "JOIN" }, { id: "DECLINE" }] } },
    expect: "DECLINE",
  },
];

/** 自检：13 场景 golden 表与后端语义一致性断言；不一致即 throw（smoke 启动时调用）。 */
export function selfcheck() {
  for (const g of GOLDEN) {
    const got = defaultDecision(g.point).option_id;
    if (got !== g.expect) {
      throw new Error(`shared_defaults 自检失败 [${g.name}]：期望 ${g.expect}，实际 ${got}——请同步 decide.ts 与后端 source.rs`);
    }
  }
}

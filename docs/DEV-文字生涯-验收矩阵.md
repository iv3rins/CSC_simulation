# 验收矩阵 · 16:30交付窗口

原需求I01–I10与T00–T12继续以analysis/26为准。本次部分实施不降低最终标准。

|需求|反例/输入|要求|当前负责|
|---|---|---|---|
|批次事务|collect后非法/漏项/业务失败|pending可再答、world/VRS/RNG/log全不变|execution|
|分支|团队选择后solo路径|不可触发；后继和事实同时满足|narrative|
|承诺|now10 due999|不触发|narrative|
|资产|重复option、ghost after_scene、自循环|显式错误且定位场景|narrative|
|效果|缺演员/承诺、组合cohesion、上限|失败全不变；成功outcome来自实际差值|narrative|
|序章幂等|提交后重复同身份载荷|完全相同receipt且日志只一次|主线程|
|身份冲突|同request不同choice/scene/generation|拒绝，无模拟副作用|主线程|
|存盘故障|不存在可用存档目录/写入失败|不能返回committed，活状态不变|主线程|
|正常重启|已落盘包重装后重试|同receipt|主线程|
|旧档载入|load后旧generation请求|拒绝|主线程|
|世界暂停|比赛/日月嵌套Awaiting|本窗口未完成，不开放story世界推进|execution/主线程|
|前端|六页/真实DTO/无world能力|阅读优先；无假数据和auto代答|frontend|

日志保存docs/analysis/implementation-2026-09-11-*.log，截图如可用保存docs/screenshots/。每项最终由实际测试覆盖记录验收，不能以清单本身代替执行证据。

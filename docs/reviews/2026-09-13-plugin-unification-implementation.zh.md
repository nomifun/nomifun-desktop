# 插件统一实现记录

日期：2026-09-13。依据：`2026-09-12-plugin-miniapp-unification-feasibility.zh.md` 及本次“一切皆插件”的实施要求。

## 已落实的变化

- 删除独立小程序导航和 `/mini-apps` 页面路由；插件首页统一显示已安装能力、可打开的页面、后台插件、草稿和回收站条目。
- 创建入口接收自然语言需求，不让用户选择“插件还是小程序”。页面、后台服务和可供对话调用的动作由生成结果组合。
- 同一个导入入口识别能力包、页面、发布包和备份；新的单文件导出扩展名为 `.nomiplugin`。
- 固定到侧栏使用共同的插件工作区；安装型能力和发布型插件共用所有权检查及删除后的文档清理。
- 将 `nomifun-plugin-service` 与 `nomifun-miniapp-platform` 的实现、资源和测试移入 `nomifun-plugin-platform`，删除两个旧 crate 及其依赖。`application` 承担共享能力 Host 的安装职责，`runtime` 承担发布、页面、独立服务和受管数据职责。
- 所有产品 HTTP 路径收敛到 `/api/plugins` 下。公共 DTO 使用 `plugin`、`plugin_id`、`plugin_runtime` 等字段；不提供旧 HTTP 路由或旧字段别名。CLI 删除顶层 `miniapp` 命令，运行信息从 `plugin runtime` 查询。
- UI/API 契约版本从 27 升至 28，避免旧前端继续使用已退出的接口。

## 能力组合

发布 Manifest 的页面描述变为可选项。后台插件可以只包含 `service/main.mjs`；页面文件必须有对应声明，不能以无效或孤立页面资源绕过校验。没有页面的插件不能申请页面会话，也不会因为错误的打开请求而启动后台服务。

源码中的 `nomifun.plugin.json` 纳入源码快照和构建摘要。构建读取动作、完整 capability contributions、schema、凭据槽、资源需求、文件和数据库需求、迁移以及服务生命周期，替代原先把这些字段硬编码为空的行为。例如：

```json
{
  "lifecycle": "on_demand",
  "actions": [
    {
      "id": "echo",
      "name": "回显",
      "description": "返回输入的对象",
      "input_schema": { "type": "object" },
      "output_schema": { "type": "object" },
      "effect": "pure"
    }
  ]
}
```

后台模块导出 `start(context)`，返回的 `invoke({ method, payload, signal })` 处理动作。构建器生成绑定到当前插件身份和 schema digest 的 capability 声明。页面可以通过 `window.nomi.service.invoke(method, payload)` 使用同一个后台。

预览使用临时存储，`window.nomi.preview` 为 `true`；后台调用会明确拒绝，不会返回虚假的成功结果。产品保存流程对后台版本执行 Service Test，通过后才发布和启用，不自动忽略测试警告。

## 数据与运行约束

迁移 `094_plugin_product_documents.sql` 将工作区和草稿迁入 `plugin_product_documents`，转换草稿的插件关联字段，同时保留 owner、内容、版本号和时间。运行型和安装型插件的数据删除复用同一个事务内文档清理函数。

已发布 Artifact 的内容、摘要、发布 epoch、Service run key、页面会话和所有权约束继续有效。嵌入不可变 HTML 的 v1 MessageChannel 握手标识保留原值；它是版本化传输合同，不是用户产品入口。新增边界测试防止前后端分别改名导致已有页面无法握手。

共享能力 Host、独立 Service Host、页面沙箱及受管存储保持各自的执行边界。统一产品不等于取消凭据、所有权、CAS 或发布准入校验。

## 验证

- 插件平台单元和集成测试，包括源码构建、独立服务、存储、发布、备份及删除恢复。
- 新增无页面后台插件构建测试：验证最终产物包含对话能力且不包含 UI，拒绝其他 owner 修改。
- 新增文档迁移、安装型插件所有权及事务删除清理测试。
- 应用层插件路由、创作保存和 HTTP 生命周期测试；公共 API 序列化及 CLI 测试。
- 前端类型、交互、接口映射、导航、国际化及生产构建检查。
- 浏览器使用实际首页组件和测试条目检查布局、搜索及交互标签；这不是已安装桌面客户端或真实 AI 模型的完整验收。

## 尚未完成的深层收敛

当前改动完成了产品入口、平台模块、公共接口、创作能力组合和工作区的合并，但 **N1 安装聚合与 M1 发布聚合仍保留各自的数据根及底层版本化机器契约**。这些没有通过改名假装变成同一个持久化聚合。

如果验收标准是“整个后端只剩一套身份、Project、Artifact、发布指针和生命周期模型，物理退出所有旧聚合”，本次还没有达到该标准。后续需要继续统一这些领域对象及事务状态机，并完成现有 Artifact、数据目录、发布证明和恢复流程的迁移验证，才能宣称整个后端的零历史债合并完成。

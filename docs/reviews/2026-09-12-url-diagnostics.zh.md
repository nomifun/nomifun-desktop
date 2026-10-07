# R20 审计记录：URL 诊断脱敏整合

日期：2026-09-12。续接入口：[全局台账](audit-progress.zh.md)。

## R20-01：公共规则与跨层重复

三项新增回归先失败：括号查询泄漏后续凭据；公共实现不处理 userinfo/非 HTTP
协议/fragment；多 URL 与重复脱敏不稳定。现在公共函数处理显式 scheme:// URL，
统一去除 userinfo、query、fragment 敏感内容，保留 host/route、尾部括号和原因。
识别自己的替换标记，避免再次脱敏时把它当 HTML 边界后重复追加标记。

保留公共函数原名称和调用签名。删除 model-invoke 的 strip_url_userinfo、
strip_non_http_url_queries 和串接包装，直接导入公共函数作为 transport_cause_detail；
删除 Agent send_error 的单行转发包装。未知凭据的 URL 防护不再只存在于模型层。

## 验证及 R20-02 待办

- 网络最终 61/0、1 子进程夹具 ignored；provider retryable_tests 19/0。
- model-invoke error::tests 11/0，transport::tests::error_from_response 4/0。
- Agent protocol::send_error 36/0；未改变错误分类、原因前缀与字数上限。
- 扩大的 model-invoke 全量 lib 测试在多项 service/数据库相关测试长时间高 CPU
  未结束，已核对进程身份后停止本次测试进程，未停止其他任务。该次不计通过；
  R20-02 单独登记，尚不能归因于本批 URL 修改。定向验证均在整合后通过。
- 未将调用方整个模块标为已审计。精确凭据的混合编码与共享客户端待续审。

# 全双工生产协议参考

2026-10-07更新：本文只保留供应商协议依据，不再是Desktop产品方案或主Agent schema/历史重构任务指令。当前唯一产品实施方案为2026-10-07-mobile-full-duplex-voice-implementation.zh.md。先前完整工作稿在本任务已校验的忽略快照中，不作为当前开发或生成器输入。

两生产adapter共享独立voice ports/core，wire在model-invoke/voice。原文字、batch ASR/TTS、robot和创作路径保最新upstream。协议能力、账号/模型权限、本地合同、实际声学体验分别列证据；不能从型号、audio I/O或握手推导语义双工验收。

- StepAudio 3：typed工具和session.update/input_audio_buffer协议；[模型说明](https://platform.stepfun.com/docs/zh/guides/models/stepaudio-3-realtime)、[API](https://platform.stepfun.com/docs/zh/api-reference/realtime/chat)。VAD使用显式adapter配置，不把供应商默认值固化到core。
- GPT-Live：[委托机制](https://developers.openai.com/api/docs/guides/live-delegation)、[WebSocket](https://developers.openai.com/api/docs/guides/voice-websockets)、[WebRTC](https://developers.openai.com/api/docs/guides/voice-webrtc)。metadata delegation只有应用结合真实转写与冻结工作上下文才能形成明确任务，不能伪造typed tool参数。
- 初始canonical事实与voice-local恢复采用各adapter合法静默注入及匹配ACK。恢复仅当前scope内committed用户输入与实际消费timing，不迁移provider私有item、权限或助理全文；source撤回先过滤缓存，再打断/重建。

实际厂商权限、设备/声学及OS验收见当前acceptance，缺凭据不能标已验收。第三方适配器应只依赖公开voice-core/voice-contracts，并用tests/fixtures/voice-external-adapter验证公开端口，不依赖应用或供应商内部实现。

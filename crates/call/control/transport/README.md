# call/transport

`call/transport` 现在不再承载 LiveKit 实现。

它只保留非当前主线或历史 transport 目录，例如：
- `trtc/`
- `webrtc/`

规则：
- `call` 的 transport 子树不应再次引入 LiveKit-specific 代码
- 任何 LiveKit-specific 的实现、类型、token、runtime adapter 都应放在 `rtc`
- `call` application 不得直接依赖这里的 provider SDK 细节

// Isolated remote experiment only. Included inside native-entry-capture.cc's
// anonymous namespace so both transports use the identical capture helper.
// This owns a separate Runtime-only Inspector session; it does not replace
// Node's debugger delegate. No replay path loads this experimental build.
// GetInspector's private ABI is pinned to Node 24.18.0. Failure to find the
// exported symbol is an unavailable experiment, never a memory-layout guess.
class NativeInspectorCapture final : public v8_inspector::V8Inspector::Channel {
 public:
  explicit NativeInspectorCapture(Isolate* isolate) : isolate_(isolate) {}
  ~NativeInspectorCapture() override { Close(); }

  bool Start(Local<Context> context, Local<String> nonce, Local<Function> sink) {
    using GetInspector = v8_inspector::V8Inspector* (*)(Isolate*);
    auto get_inspector = reinterpret_cast<GetInspector>(
        dlsym(RTLD_DEFAULT, "_ZN2v85debug12GetInspectorEPNS_7IsolateE"));
    if (!get_inspector) { failure_ = "native inspector accessor unavailable"; return false; }
    auto inspector = get_inspector(isolate_);
    if (!inspector) { failure_ = "existing inspector unavailable"; return false; }
    Local<Function> capture;
    if (!Function::New(context, CaptureOrdinaryPacked, nonce, 1,
                       ConstructorBehavior::kThrow).ToLocal(&capture)) return false;
    context_.Reset(isolate_, context);
    capture_.Reset(isolate_, capture);
    sink_.Reset(isolate_, sink);
    // Node's pinned inspector_agent.cc uses context group 1 for this isolate.
    session_ = inspector->connect(1, this, {},
                                  v8_inspector::V8Inspector::kFullyTrusted);
    if (!session_) { failure_ = "native inspector connection unavailable"; return false; }
    const char command[] = "{\"id\":1,\"method\":\"Runtime.enable\"}";
    session_->dispatchProtocolMessage(View(command));
    return enabled_ && failure_ == nullptr;
  }

  static v8_inspector::StringView View(const char* value) {
    return {reinterpret_cast<const uint8_t*>(value), std::char_traits<char>::length(value)};
  }

  // JSON.parse creates own data properties. Still reject missing/accessor
  // fields explicitly rather than falling through a modified prototype.
  static bool Field(Local<Context> context, Local<Value> owner,
                    const char* name, Local<Value>* result) {
    if (!owner->IsObject() || owner->IsProxy()) return false;
    auto object = owner.As<Object>();
    auto key = Key(context->GetIsolate(), name);
    if (!object->HasRealNamedProperty(context, key).FromMaybe(false) ||
        object->HasRealNamedCallbackProperty(context, key).FromMaybe(true)) return false;
    return object->GetRealNamedProperty(context, key).ToLocal(result);
  }

  MaybeLocal<Value> Parse(v8_inspector::StringView message) {
    Local<String> text;
    if (message.length() > static_cast<size_t>(String::kMaxLength)) return {};
    bool okay = message.is8Bit()
        ? String::NewFromOneByte(isolate_, message.characters8(), NewStringType::kNormal,
                                static_cast<int>(message.length())).ToLocal(&text)
        : String::NewFromTwoByte(isolate_, message.characters16(), NewStringType::kNormal,
                                static_cast<int>(message.length())).ToLocal(&text);
    if (!okay) return {};
    return JSON::Parse(context_.Get(isolate_), text);
  }

  void sendResponse(int call_id, std::unique_ptr<v8_inspector::StringBuffer> message) override {
    HandleScope handles(isolate_);
    Local<Value> root, error;
    if (call_id != 1 || !Parse(message->string()).ToLocal(&root) ||
        Field(context_.Get(isolate_), root, "error", &error)) {
      failure_ = "native runtime enable failed";
      return;
    }
    enabled_ = true;
  }

  void sendNotification(std::unique_ptr<v8_inspector::StringBuffer> message) override {
    if (closed_) return;
    if (processing_) { Fail("reentrant native notification"); return; }
    processing_ = true;
    struct ProcessingGuard { bool& value; ~ProcessingGuard() { value = false; } } guard{processing_};
    HandleScope handles(isolate_);
    auto context = context_.Get(isolate_);
    Context::Scope context_scope(context);
    MicrotasksScope microtasks(isolate_, context->GetMicrotaskQueue(), MicrotasksScope::kDoNotRunMicrotasks);
    TryCatch caught(isolate_);
    Local<Value> root, method;
    if (!Parse(message->string()).ToLocal(&root) || !Field(context, root, "method", &method) ||
        !method->IsString()) { Fail("invalid native notification"); return; }
    if (!method->StrictEquals(Key(isolate_, "Runtime.exceptionThrown"))) return;
    ++notifications_;
    Local<Value> params, details, remote, id, subtype;
    bool had_handle = false;
    do {
      if (!Field(context, root, "params", &params) || !Field(context, params, "exceptionDetails", &details) ||
          !Field(context, details, "exception", &remote)) { Fail("exception metadata unavailable"); break; }
      if (!Field(context, remote, "objectId", &id)) { ++ignored_; break; }
      if (!id->IsString()) { Fail("invalid exception handle"); break; }
      if (Field(context, remote, "subtype", &subtype) &&
          (subtype->StrictEquals(Key(isolate_, "proxy")) || subtype->StrictEquals(Key(isolate_, "error")))) {
        ++ignored_; break;
      }
      String::Value object_id(isolate_, id);
      std::unique_ptr<v8_inspector::StringBuffer> error, group;
      Local<Value> payload;
      Local<Context> payload_context;
      if (!session_->unwrapObject(&error,
              {reinterpret_cast<const uint16_t*>(*object_id), static_cast<size_t>(object_id.length())},
              &payload, &payload_context, &group) || payload_context.IsEmpty()) {
        Fail("native exception unwrap failed"); break;
      }
      ++unwrapped_;
      had_handle = true;
      Local<Value> args[] = {payload}, summary;
      if (!capture_.Get(isolate_)->Call(context, Undefined(isolate_), 1, args).ToLocal(&summary)) {
        Fail("native capture helper failed"); break;
      }
      // Only the helper's sanitized packed result enters the JS observer.
      Local<Value> sink_args[] = {summary}, ignored_result;
      if (!sink_.Get(isolate_)->Call(context, Undefined(isolate_), 1, sink_args).ToLocal(&ignored_result)) {
        Fail("native summary callback failed"); break;
      }
      ++delivered_;
    } while (false);
    if (caught.HasCaught()) { caught.Reset(); Fail("native notification exception"); }
    // Handles belong only to this private session. Global console storage may
    // retain the payload, whose application references the helper already clears.
    session_->releaseObjectGroup(View("console"));
    ++releases_;
    if (had_handle) {
      String::Value object_id(isolate_, id);
      std::unique_ptr<v8_inspector::StringBuffer> error, group;
      Local<Value> released;
      Local<Context> released_context;
      if (session_->unwrapObject(&error,
              {reinterpret_cast<const uint16_t*>(*object_id), static_cast<size_t>(object_id.length())},
              &released, &released_context, &group)) Fail("native exception handle retained after release");
      else ++verified_releases_;
    }
  }

  void flushProtocolNotifications() override {}

  void Fail(const char* reason) { ++failures_; if (!failure_) failure_ = reason; }

  void Close() {
    if (closed_) return;
    if (processing_) { Fail("close requested during native notification"); return; }
    closed_ = true;
    session_.reset();
    sink_.Reset(); capture_.Reset(); context_.Reset();
  }

  Local<Object> Report(Local<Context> context) {
    auto result = Object::New(isolate_);
    (void)Put(context, result, "enabled", Boolean::New(isolate_, enabled_));
    (void)Put(context, result, "closed", Boolean::New(isolate_, closed_));
    (void)Put(context, result, "notifications", Number::New(isolate_, notifications_));
    (void)Put(context, result, "unwrapped", Number::New(isolate_, unwrapped_));
    (void)Put(context, result, "delivered", Number::New(isolate_, delivered_));
    (void)Put(context, result, "ignored", Number::New(isolate_, ignored_));
    (void)Put(context, result, "groupsReleased", Number::New(isolate_, releases_));
    (void)Put(context, result, "verifiedHandleReleases", Number::New(isolate_, verified_releases_));
    (void)Put(context, result, "failures", Number::New(isolate_, failures_));
    (void)Put(context, result, "failure", failure_ ? Local<Value>(Key(isolate_, failure_)) : Local<Value>(Null(isolate_)));
    (void)Put(context, result, "replayEligible", False(isolate_));
    return result;
  }

 private:
  Isolate* isolate_;
  Global<Context> context_;
  Global<Function> capture_, sink_;
  std::unique_ptr<v8_inspector::V8InspectorSession> session_;
  const char* failure_ = nullptr;
  uint64_t notifications_ = 0, unwrapped_ = 0, delivered_ = 0, ignored_ = 0, releases_ = 0, failures_ = 0;
  uint64_t verified_releases_ = 0;
  bool enabled_ = false, closed_ = false, processing_ = false;
};

void InspectorCaptureReport(const FunctionCallbackInfo<Value>& info) {
  auto observer = static_cast<NativeInspectorCapture*>(info.Data().As<External>()->Value());
  info.GetReturnValue().Set(observer->Report(info.GetIsolate()->GetCurrentContext()));
}
void InspectorCaptureClose(const FunctionCallbackInfo<Value>& info) {
  static_cast<NativeInspectorCapture*>(info.Data().As<External>()->Value())->Close();
}
void CreateInspectorCapture(const FunctionCallbackInfo<Value>& info) {
  auto isolate = info.GetIsolate();
  auto context = isolate->GetCurrentContext();
  if (info.Length() != 2 || !info[0]->IsString() || info[0].As<String>()->Length() != 64 || !info[1]->IsFunction()) {
    isolate->ThrowException(Exception::TypeError(Key(isolate, "invalid native inspector experiment arguments")));
    return;
  }
  auto observer = new NativeInspectorCapture(isolate);
  bool started = observer->Start(context, info[0].As<String>(), info[1].As<Function>());
  if (!started) {
    auto report = observer->Report(context);
    delete observer;
    info.GetReturnValue().Set(report);
    return;
  }
  node::AddEnvironmentCleanupHook(isolate, [](void* value) { delete static_cast<NativeInspectorCapture*>(value); }, observer);
  auto result = Object::New(isolate);
  auto data = External::New(isolate, observer);
  Local<Function> report, close;
  if (Function::New(context, InspectorCaptureReport, data).ToLocal(&report) &&
      Function::New(context, InspectorCaptureClose, data).ToLocal(&close) &&
      Put(context, result, "report", report) && Put(context, result, "close", close)) info.GetReturnValue().Set(result);
}

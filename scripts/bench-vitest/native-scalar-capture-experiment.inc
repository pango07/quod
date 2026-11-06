// Included only in the diagnostic addon. No process hooks or replay authority.
// State stays in V8-owned objects so target/callback cycles remain collectible.
Local<Value> ScalarState(Local<Context> context, Local<Object> state,
                         const char* name) {
  Local<Value> value;
  if (!state->Get(context, Key(context->GetIsolate(), name)).ToLocal(&value))
    return Undefined(context->GetIsolate());
  return value;
}
bool ScalarFlag(Local<Context> context, Local<Object> state, const char* name) {
  return ScalarState(context, state, name)->IsTrue();
}
void ScalarMark(Local<Context> context, Local<Object> state, const char* name,
                bool value = true) {
  (void)Put(context, state, name, Boolean::New(context->GetIsolate(), value));
}

void NativeScalarRead(Local<Name>, const PropertyCallbackInfo<Value>& info) {
  auto isolate = info.GetIsolate();
  auto context = isolate->GetCurrentContext();
  auto state = info.Data().As<Object>();
  auto value = ScalarState(context, state, "value");
  info.GetReturnValue().Set(value);
  if (ScalarFlag(context, state, "closed")) return;
  if (ScalarFlag(context, state, "checking")) {
    ScalarMark(context, state, "witness");
    return;
  }
  auto calls = ScalarState(context, state, "calls");
  if (!calls->IsNumber() || calls.As<Number>()->Value() >= 9007199254740991.0) {
    ScalarMark(context, state, "failed");
    return;
  }
  (void)Put(context, state, "calls", Number::New(isolate, calls.As<Number>()->Value() + 1));
  if (ScalarFlag(context, state, "inFlight")) {
    ScalarMark(context, state, "failed");
    return;
  }
  if (ScalarFlag(context, state, "failed")) return;
  auto sink = ScalarState(context, state, "sink");
  if (!sink->IsFunction()) { ScalarMark(context, state, "failed"); return; }
  ScalarMark(context, state, "inFlight");
  {
    // A diagnostic callback must neither throw into the application read nor
    // advance queued microtasks at this new native-to-JS boundary.
    TryCatch caught(isolate);
    MicrotasksScope microtasks(isolate, context->GetMicrotaskQueue(), MicrotasksScope::kDoNotRunMicrotasks);
    Local<Value> ignored;
    bool okay = sink.As<Function>()->Call(context, Undefined(isolate), 0, nullptr).ToLocal(&ignored);
    if (!okay || caught.HasCaught()) {
      caught.Reset();
      ScalarMark(context, state, "failed");
    }
  }
  ScalarMark(context, state, "inFlight", false);
}

bool NativeScalarIntact(Local<Context> context, Local<Object> state) {
  if (ScalarFlag(context, state, "closed")) return false;
  if (ScalarFlag(context, state, "checking") || ScalarFlag(context, state, "inFlight")) {
    ScalarMark(context, state, "failed");
    return false;
  }
  auto owner = ScalarState(context, state, "owner");
  auto key = ScalarState(context, state, "key");
  if (!OrdinaryForInspection(owner) || !key->IsString()) return false;
  auto target = owner.As<Object>();
  auto callback = target->HasRealNamedCallbackProperty(context, key.As<String>());
  if (callback.IsNothing() || !callback.FromJust()) return false;
  ScalarMark(context, state, "checking");
  ScalarMark(context, state, "witness", false);
  bool intact = false;
  {
    TryCatch caught(context->GetIsolate());
    Local<Value> descriptor;
    if (target->GetOwnPropertyDescriptor(context, key.As<String>()).ToLocal(&descriptor) &&
        descriptor->IsObject() && ScalarFlag(context, state, "witness")) {
      auto object = descriptor.As<Object>();
      Local<Value> current;
      if (object->HasOwnProperty(context, Key(context->GetIsolate(), "value")).FromMaybe(false) &&
          object->Get(context, Key(context->GetIsolate(), "value")).ToLocal(&current))
        intact = current->SameValue(ScalarState(context, state, "value"));
    }
    if (caught.HasCaught()) { caught.Reset(); ScalarMark(context, state, "failed"); }
  }
  ScalarMark(context, state, "checking", false);
  return intact;
}

void NativeScalarReport(const FunctionCallbackInfo<Value>& info) {
  auto isolate = info.GetIsolate();
  auto context = isolate->GetCurrentContext();
  auto state = info.Data().As<Object>();
  bool intact = NativeScalarIntact(context, state);
  bool closed = ScalarFlag(context, state, "closed"), failed = ScalarFlag(context, state, "failed");
  auto result = Object::New(isolate);
  if (Put(context, result, "installed", Boolean::New(isolate, !closed)) &&
      Put(context, result, "intact", Boolean::New(isolate, intact)) &&
      Put(context, result, "available", Boolean::New(isolate, intact && !failed && !closed)) &&
      Put(context, result, "calls", ScalarState(context, state, "calls")) &&
      Put(context, result, "failed", Boolean::New(isolate, failed)) &&
      Put(context, result, "closed", Boolean::New(isolate, closed)) &&
      Put(context, result, "restored", Boolean::New(isolate, ScalarFlag(context, state, "restored"))) &&
      Put(context, result, "retainedAfterClose", Boolean::New(isolate, ScalarFlag(context, state, "retained"))) &&
      Put(context, result, "replayEligible", False(isolate))) info.GetReturnValue().Set(result);
}

void NativeScalarCurrent(const FunctionCallbackInfo<Value>& info) {
  auto isolate = info.GetIsolate();
  auto context = isolate->GetCurrentContext();
  auto state = info.Data().As<Object>();
  auto result = Object::New(isolate);
  bool available = NativeScalarIntact(context, state) && !ScalarFlag(context, state, "failed");
  Maybe<PropertyAttribute> attributes = Nothing<PropertyAttribute>();
  if (available) {
    attributes = ScalarState(context, state, "owner").As<Object>()->GetRealNamedPropertyAttributes(
        context, ScalarState(context, state, "key").As<String>());
    available = attributes.IsJust();
  }
  if (!Put(context, result, "available", Boolean::New(isolate, available))) return;
  if (available && (!Put(context, result, "value", ScalarState(context, state, "value")) ||
      !Put(context, result, "enumerable", Boolean::New(isolate, (attributes.FromJust() & DontEnum) == 0)) ||
      !Put(context, result, "writable", Boolean::New(isolate, (attributes.FromJust() & ReadOnly) == 0)) ||
      !Put(context, result, "configurable", Boolean::New(isolate, (attributes.FromJust() & DontDelete) == 0)))) return;
  info.GetReturnValue().Set(result);
}

void NativeScalarClose(const FunctionCallbackInfo<Value>& info) {
  auto isolate = info.GetIsolate();
  auto context = isolate->GetCurrentContext();
  auto state = info.Data().As<Object>();
  if (ScalarFlag(context, state, "closed")) return;
  if (ScalarFlag(context, state, "inFlight") || ScalarFlag(context, state, "checking")) {
    ScalarMark(context, state, "failed");
    return;
  }
  bool intact = NativeScalarIntact(context, state);
  ScalarMark(context, state, "closed");
  (void)Put(context, state, "sink", Null(isolate));
  if (intact) {
    auto owner = ScalarState(context, state, "owner").As<Object>();
    auto key = ScalarState(context, state, "key").As<String>();
    TryCatch caught(isolate);
    auto attributes = owner->GetRealNamedPropertyAttributes(context, key);
    bool restored = false;
    if (attributes.IsJust() && (attributes.FromJust() & DontDelete) == 0) {
      PropertyDescriptor descriptor(ScalarState(context, state, "value"), (attributes.FromJust() & ReadOnly) == 0);
      descriptor.set_enumerable((attributes.FromJust() & DontEnum) == 0);
      descriptor.set_configurable(true);
      restored = owner->DefineProperty(context, key, descriptor).FromMaybe(false);
    }
    if (caught.HasCaught()) { caught.Reset(); ScalarMark(context, state, "failed"); }
    ScalarMark(context, state, "restored", restored);
    ScalarMark(context, state, "retained", !restored);
  }
  // Frozen properties may retain the closed native callback. It only returns
  // the original scalar; it owns neither the application owner nor JS sink.
  (void)Put(context, state, "owner", Null(isolate));
}

void CreateNativeScalarObserver(const FunctionCallbackInfo<Value>& info) {
  auto isolate = info.GetIsolate();
  auto context = isolate->GetCurrentContext();
  Local<Value> value;
  Maybe<PropertyAttribute> attributes = Nothing<PropertyAttribute>();
  bool okay = info.Length() == 3 && OrdinaryForInspection(info[0]) && info[1]->IsString() && info[2]->IsFunction();
  if (okay) {
    auto target = info[0].As<Object>();
    auto key = info[1].As<String>();
    auto own = target->HasOwnProperty(context, key);
    auto callback = target->HasRealNamedCallbackProperty(context, key);
    attributes = target->GetRealNamedPropertyAttributes(context, key);
    okay = own.IsJust() && own.FromJust() && callback.IsJust() && !callback.FromJust() &&
        attributes.IsJust() && (attributes.FromJust() & (ReadOnly | DontDelete)) == 0 &&
        target->GetRealNamedProperty(context, key).ToLocal(&value) && !value->IsObject();
  }
  if (!okay) {
    isolate->ThrowException(Exception::TypeError(Key(isolate, "unsupported native scalar owner or field")));
    return;
  }
  auto state = Object::New(isolate), result = Object::New(isolate);
  if (!state->SetPrototypeV2(context, Null(isolate)).FromMaybe(false) ||
      !Put(context, state, "owner", info[0]) || !Put(context, state, "key", info[1]) ||
      !Put(context, state, "value", value) || !Put(context, state, "sink", info[2]) ||
      !Put(context, state, "calls", Number::New(isolate, 0))) return;
  for (const char* flag : {"closed", "failed", "inFlight", "checking", "witness", "restored", "retained"})
    ScalarMark(context, state, flag, false);
  Local<Function> report, close, current;
  if (!Function::New(context, NativeScalarReport, state, 0, ConstructorBehavior::kThrow).ToLocal(&report) ||
      !Function::New(context, NativeScalarClose, state, 0, ConstructorBehavior::kThrow).ToLocal(&close) ||
      !Function::New(context, NativeScalarCurrent, state, 0, ConstructorBehavior::kThrow).ToLocal(&current) ||
      !Put(context, result, "report", report) || !Put(context, result, "close", close) ||
      !Put(context, result, "currentValue", current)) return;
  if (!info[0].As<Object>()->SetNativeDataProperty(context, info[1].As<String>(), NativeScalarRead,
      nullptr, state, attributes.FromJust()).FromMaybe(false)) return;
  info.GetReturnValue().Set(result);
}

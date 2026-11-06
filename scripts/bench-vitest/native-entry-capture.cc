// Diagnostic only, built against the pinned Node headers on GitHub.
// No process-wide hooks, persistent application references, or public bridge.
// This helper does not certify initial runtime integrity or replay eligibility.
#include <node.h>
#include <cmath>
#include <initializer_list>

namespace {
using namespace v8;

Local<String> Key(Isolate* isolate, const char* value) {
  return String::NewFromUtf8(isolate, value, NewStringType::kInternalized)
      .ToLocalChecked();
}

bool OrdinaryForInspection(Local<Value> value) {
  if (!value->IsObject() || value->IsProxy()) return false;
  auto object = value.As<Object>();
  return !object->IsApiWrapper() && !object->HasNamedLookupInterceptor() &&
         !object->HasIndexedLookupInterceptor();
}

bool OwnData(Local<Context> context, Local<Object> object, const char* name,
             Local<Value>* value, bool writable = false) {
  auto isolate = context->GetIsolate();
  // A native data callback can masquerade as a data descriptor and execute
  // while GetOwnPropertyDescriptor fills its value. Owned fields must be real
  // data, so reject every callback before requesting their descriptors.
  auto callback = object->HasRealNamedCallbackProperty(context, Key(isolate, name));
  if (callback.IsNothing() || callback.FromJust()) return false;
  Local<Value> descriptor_value;
  if (!object->GetOwnPropertyDescriptor(context, Key(isolate, name))
           .ToLocal(&descriptor_value) || !descriptor_value->IsObject())
    return false;
  auto descriptor = descriptor_value.As<Object>();
  if (!descriptor->HasOwnProperty(context, Key(isolate, "value"))
           .FromMaybe(false) ||
      !descriptor->Get(context, Key(isolate, "value")).ToLocal(value))
    return false;
  if (writable) {
    Local<Value> flag;
    if (!descriptor->Get(context, Key(isolate, "writable")).ToLocal(&flag) ||
        !flag->IsTrue()) return false;
  }
  return true;
}

bool HasOwnAccessor(Local<Context> context, Local<Object> object,
                    const char* name, bool* found) {
  // This V8 own-property query skips interceptors and does not fetch values.
  // Conservatively include native data callbacks and empty JS accessors too.
  auto callback = object->HasRealNamedCallbackProperty(
      context, Key(context->GetIsolate(), name));
  if (callback.IsNothing()) return false;
  *found = callback.FromJust();
  return true;
}

bool Put(Local<Context> context, Local<Object> result, const char* name,
         Local<Value> value) {
  return result->CreateDataProperty(context, Key(context->GetIsolate(), name),
                                    value).FromMaybe(false);
}

void Capture(const FunctionCallbackInfo<Value>& info) {
  auto isolate = info.GetIsolate();
  HandleScope scope(isolate);
  auto context = isolate->GetCurrentContext();
  TryCatch caught(isolate);
  auto result = Object::New(isolate);
  if (!result->SetPrototypeV2(context, Null(isolate)).FromMaybe(false)) return;
  bool owned = false, available = true, clearable = false, cleared = false;
  bool inspected = false, receiver = false, describe = false;
  int kind = 0, mask = 0;  // 0 primitive, 1 ordinary object, 2 proxy.
  int rejection = 0;  // Foreign-wrapper classification, never target data.
  double count = 0;
  Local<Object> payload;
  Local<Value> api = Undefined(isolate);
  do {
    if (info.Length() != 1) { rejection = 1; break; }
    if (!OrdinaryForInspection(info[0])) { rejection = 2; break; }
    payload = info[0].As<Object>();
    if (!payload->GetPrototypeV2()->IsNull()) { rejection = 3; break; }
    Local<Value> marker;
    if (!OwnData(context, payload, "quodNativeEntry", &marker)) {
      rejection = 4; break;
    }
    if (!marker->StrictEquals(info.Data())) { rejection = 5; break; }
    owned = true;
    available = false;
    Local<Value> argument, count_value, receiver_value, describe_value;
    if (!OwnData(context, payload, "quodNativeEntry", &marker, true) ||
        !OwnData(context, payload, "api", &api, true) ||
        !OwnData(context, payload, "argument", &argument, true) ||
        !OwnData(context, payload, "argumentCount", &count_value, true) ||
        !OwnData(context, payload, "receiverBased", &receiver_value, true) ||
        !OwnData(context, payload, "describeOwnAccessors", &describe_value, true))
      break;
    clearable = true;
    if (!api->IsString() || !count_value->IsNumber() ||
        !receiver_value->IsBoolean() || !describe_value->IsBoolean()) break;
    count = count_value.As<Number>()->Value();
    if (!std::isfinite(count) || count < 0 || count > 9007199254740991.0 ||
        std::floor(count) != count) break;
    receiver = receiver_value->IsTrue();
    describe = describe_value->IsTrue();
    kind = argument->IsProxy() ? 2 : argument->IsObject() ? 1 : 0;
    if (describe && kind == 1) {
      if (!OrdinaryForInspection(argument)) break;
      const char* names[] = {"seed", "tmp", "invalidate"};
      bool okay = true;
      for (int index = 0; index < 3; ++index) {
        bool accessor = false;
        if (!HasOwnAccessor(context, argument.As<Object>(), names[index],
                            &accessor)) { okay = false; break; }
        if (accessor) mask |= 1 << index;
      }
      if (!okay) break;
      inspected = true;
    }
    available = true;
  } while (false);

  if (caught.HasCaught()) { caught.Reset(); available = false; }
  if (owned && clearable) {
    // Only our own validated writable data fields are modified. Even an
    // unsupported target loses its references in retained console history.
    cleared = true;
    for (const char* field : {"argument", "api", "quodNativeEntry"})
      cleared = payload->Set(context, Key(isolate, field), Null(isolate))
                    .FromMaybe(false) && cleared;
    if (caught.HasCaught()) { caught.Reset(); cleared = false; }
    available = available && cleared;
  }
  bool okay = Put(context, result, "owned", Boolean::New(isolate, owned)) &&
      Put(context, result, "available", Boolean::New(isolate, available)) &&
      Put(context, result, "cleared", Boolean::New(isolate, cleared)) &&
      Put(context, result, "rejection", Integer::New(isolate, rejection));
  if (owned && available) {
    okay = okay && Put(context, result, "api", api) &&
        Put(context, result, "argumentCount", Number::New(isolate, count)) &&
        Put(context, result, "receiverBased", Boolean::New(isolate, receiver)) &&
        Put(context, result, "describeOwnAccessors", Boolean::New(isolate, describe)) &&
        Put(context, result, "kind", Integer::New(isolate, kind)) &&
        Put(context, result, "inspected", Boolean::New(isolate, inspected)) &&
        Put(context, result, "mask", Integer::New(isolate, mask));
  }
  if (okay) info.GetReturnValue().Set(result);
}

void CreateCapture(const FunctionCallbackInfo<Value>& info) {
  auto isolate = info.GetIsolate();
  auto context = isolate->GetCurrentContext();
  if (info.Length() != 1 || !info[0]->IsString() ||
      info[0].As<String>()->Length() != 64) {
    isolate->ThrowException(Exception::TypeError(Key(isolate, "invalid nonce")));
    return;
  }
  Local<Function> capture;
  if (Function::New(context, Capture, info[0], 1, ConstructorBehavior::kThrow)
          .ToLocal(&capture)) info.GetReturnValue().Set(capture);
}

// Adversarial fixture: a host callback on an otherwise ordinary JS object.
// Reading its descriptor invokes native code even though it looks like data.
void NativeDataRead(Local<Name>, const PropertyCallbackInfo<Value>& info) {
  auto isolate = info.GetIsolate();
  auto context = isolate->GetCurrentContext();
  auto state = info.Data().As<Object>();
  Local<Value> count;
  if (!state->Get(context, Key(isolate, "calls")).ToLocal(&count)) return;
  if (!Put(context, state, "calls", Integer::New(isolate, count.As<Int32>()->Value() + 1))) return;
  info.GetReturnValue().Set(Key(isolate, "host callback value"));
}
bool NativeAccessCheck(Local<Context> context, Local<Object>, Local<Value> data) {
  auto isolate = context->GetIsolate();
  auto state = data.As<Object>();
  Local<Value> count;
  return state->Get(context, Key(isolate, "calls")).ToLocal(&count) &&
      Put(context, state, "calls", Integer::New(isolate, count.As<Int32>()->Value() + 1));
}
void CreateNativeDataFixture(const FunctionCallbackInfo<Value>& info) {
  auto isolate = info.GetIsolate();
  auto context = isolate->GetCurrentContext();
  auto target = Object::New(isolate), state = Object::New(isolate);
  auto result = Object::New(isolate);
  if (!target->SetPrototypeV2(context, Null(isolate)).FromMaybe(false) ||
      !Put(context, state, "calls", Integer::New(isolate, 0))) return;
  const char* field = info.Length() && info[0]->IsTrue() ? "quodNativeEntry" : "tmp";
  if (!target->SetNativeDataProperty(context, Key(isolate, field), NativeDataRead,
                                     nullptr, state).FromMaybe(false)) return;
  if (info.Length() > 1 && info[1]->IsTrue()) {
    auto object_template = ObjectTemplate::New(isolate);
    object_template->SetAccessCheckCallback(NativeAccessCheck, state);
    object_template->SetNativeDataProperty(Key(isolate, field), NativeDataRead,
                                           nullptr, state);
    if (!object_template->NewInstance(context).ToLocal(&target)) return;
  }
  if (Put(context, result, "target", target) && Put(context, result, "state", state) &&
      Put(context, result, "apiWrapper", Boolean::New(isolate, target->IsApiWrapper())))
    info.GetReturnValue().Set(result);
}
void Initialize(Local<Object> exports, Local<Value>, Local<Context> context,
                void*) {
  Local<Function> factory;
  if (Function::New(context, CreateCapture).ToLocal(&factory))
    (void)exports->CreateDataProperty(context, Key(context->GetIsolate(),
                                                 "createCapture"), factory)
        .FromMaybe(false);
  if (Function::New(context, CreateNativeDataFixture).ToLocal(&factory))
    (void)exports->CreateDataProperty(context, Key(context->GetIsolate(),
                                                 "createNativeDataFixture"), factory)
        .FromMaybe(false);
}
}  // namespace

NODE_MODULE_CONTEXT_AWARE(NODE_GYP_MODULE_NAME, Initialize)

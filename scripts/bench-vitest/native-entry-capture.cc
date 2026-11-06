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
  return !object->HasNamedLookupInterceptor() &&
         !object->HasIndexedLookupInterceptor();
}

bool OwnData(Local<Context> context, Local<Object> object, const char* name,
             Local<Value>* value, bool writable = false) {
  auto isolate = context->GetIsolate();
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
  auto isolate = context->GetIsolate();
  Local<Value> descriptor_value;
  if (!object->GetOwnPropertyDescriptor(context, Key(isolate, name))
           .ToLocal(&descriptor_value)) return false;
  *found = false;
  if (descriptor_value->IsUndefined()) return true;
  if (!descriptor_value->IsObject()) return false;
  auto descriptor = descriptor_value.As<Object>();
  for (const char* field : {"get", "set"}) {
    Local<Value> value;
    // Descriptor records are fresh own data. An absent field must not read
    // an application getter installed on Object.prototype.
    if (descriptor->HasOwnProperty(context, Key(isolate, field))
            .FromMaybe(false)) {
      if (!descriptor->Get(context, Key(isolate, field)).ToLocal(&value))
        return false;
      *found = *found || value->IsFunction();
    }
  }
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
  double count = 0;
  Local<Object> payload;
  Local<Value> api = Undefined(isolate);
  do {
    if (info.Length() != 1 || !OrdinaryForInspection(info[0])) break;
    payload = info[0].As<Object>();
    if (!payload->GetPrototypeV2()->IsNull()) break;
    Local<Value> marker;
    if (!OwnData(context, payload, "quodNativeEntry", &marker) ||
        !marker->StrictEquals(info.Data())) break;
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
      Put(context, result, "cleared", Boolean::New(isolate, cleared));
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
void Initialize(Local<Object> exports, Local<Value>, Local<Context> context,
                void*) {
  Local<Function> factory;
  if (Function::New(context, CreateCapture).ToLocal(&factory))
    (void)exports->CreateDataProperty(context, Key(context->GetIsolate(),
                                                 "createCapture"), factory)
        .FromMaybe(false);
}
}  // namespace

NODE_MODULE_CONTEXT_AWARE(NODE_GYP_MODULE_NAME, Initialize)

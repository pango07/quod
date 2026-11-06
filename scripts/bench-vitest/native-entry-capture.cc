// Diagnostic only, built against the pinned Node headers on GitHub.
// No process-wide hooks, persistent application references, or public bridge.
// This helper does not certify initial runtime integrity or replay eligibility.
#include <node.h>
#include <v8-profiler.h>
#include <cmath>
#include <initializer_list>
#include <vector>
#ifdef QUOD_NATIVE_INSPECTOR_EXPERIMENT
#include <dlfcn.h>
#include <string>
#include <v8-inspector.h>
#endif

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

bool OwnDataKey(Local<Context> context, Local<Object> object, Local<Name> name,
             Local<Value>* value, bool writable = false) {
  // A native data callback can masquerade as a data descriptor and execute
  // while its value is fetched. These owned payload/holder objects must have
  // no prototype, interceptor or host access check. Then the lookup cannot
  // fall through to another object, and callbacks are rejected before reads.
  if (!OrdinaryForInspection(object) ||
      !object->GetPrototypeV2()->IsNull()) return false;
  auto callback = object->HasRealNamedCallbackProperty(context, name);
  if (callback.IsNothing() || callback.FromJust()) return false;
#ifdef QUOD_CAPTURE_DESCRIPTOR_FIELDS
  // Remote microbenchmark baseline only: retain the prior allocation path.
  auto isolate = context->GetIsolate();
  Local<Value> descriptor_value;
  if (!object->GetOwnPropertyDescriptor(context, name)
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
#else
  auto attributes = object->GetRealNamedPropertyAttributes(context, name);
  if (attributes.IsNothing() ||
      (writable && (attributes.FromJust() & ReadOnly) != 0)) return false;
  return object->GetRealNamedProperty(context, name).ToLocal(value);
#endif
}

bool OwnData(Local<Context> context, Local<Object> object, const char* name,
             Local<Value>* value, bool writable = false) {
  return OwnDataKey(context, object, Key(context->GetIsolate(), name), value,
                    writable);
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

void CaptureImpl(const FunctionCallbackInfo<Value>& info, bool ordinary_owners,
                 bool packed = false, bool string_summary = false) {
  auto isolate = info.GetIsolate();
  HandleScope scope(isolate);
  auto context = isolate->GetCurrentContext();
  TryCatch caught(isolate);
  bool owned = false, available = true, clearable = false, cleared = false;
  bool inspected = false, receiver = false, describe = false, metadata_valid = false;
  int failure = 0;  // 1 wrapper, 2 API wrapper, 3 interceptor, 4 lookup, 5 exception, 6 cleanup.
  int kind = 0, mask = 0;  // 0 primitive, 1 ordinary object, 2 proxy.
  int excluded_owner = 0;  // Only valid with the ordinary-owner installation guard.
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
    failure = 1;
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
    metadata_valid = true;
    failure = 0;
    if (describe && kind == 1) {
      auto target = argument.As<Object>();
      int unsupported = target->IsApiWrapper() ? 2 :
          target->HasNamedLookupInterceptor() || target->HasIndexedLookupInterceptor() ? 3 : 0;
      if (unsupported) {
        if (ordinary_owners) { excluded_owner = unsupported - 1; available = true; }
        else failure = unsupported;
        break;
      }
      const char* names[] = {"seed", "tmp", "invalidate"};
      bool okay = true;
      for (int index = 0; index < 3; ++index) {
        bool accessor = false;
        if (!HasOwnAccessor(context, argument.As<Object>(), names[index],
                            &accessor)) { okay = false; break; }
        if (accessor) mask |= 1 << index;
      }
      if (!okay) { failure = 4; break; }
      inspected = true;
    }
    available = true;
  } while (false);

  if (caught.HasCaught()) { caught.Reset(); available = false; failure = 5; }
  if (owned && clearable) {
    // Only our own validated writable data fields are modified. Even an
    // unsupported target loses its references in retained console history.
    cleared = true;
    for (const char* field : {"argument", "api", "quodNativeEntry"})
      cleared = payload->Set(context, Key(isolate, field), Null(isolate))
                    .FromMaybe(false) && cleared;
    if (caught.HasCaught()) { caught.Reset(); cleared = false; }
    if (!cleared) failure = 6;
    available = available && cleared;
  }
  if (packed || string_summary) {
    // Versioned transport of the same sanitized summary. No application
    // values enter the response; the API label and arity are validated data
    // from our owned wrapper. Keep all rejection/failure/cleanup information.
    const int flags = static_cast<int>(owned) |
        (static_cast<int>(available) << 1) | (static_cast<int>(cleared) << 2) |
        (static_cast<int>(metadata_valid) << 3) |
        (static_cast<int>(receiver) << 4) | (static_cast<int>(describe) << 5) |
        (static_cast<int>(inspected) << 6) | (kind << 7) | (mask << 9) |
        (failure << 12) | (rejection << 15) | (excluded_owner << 18);
    Local<Value> fields[] = {Integer::New(isolate, 1),
                            Integer::New(isolate, flags),
                            owned && metadata_valid ? api : Local<Value>(Null(isolate)),
                            Number::New(isolate, owned && metadata_valid ? count : 0)};
    auto result = Array::New(isolate, fields, 4);
    if (result->SetPrototypeV2(context, Null(isolate)).FromMaybe(false)) {
      if (string_summary) {
        // No global JSON lookup and no application values: the null-prototype
        // array contains only validated scalars, so inherited toJSON hooks
        // cannot run. A primitive exception needs no Inspector object handle.
        Local<String> text;
        if (JSON::Stringify(context, result).ToLocal(&text))
          info.GetReturnValue().Set(String::Concat(isolate,
              String::Concat(isolate, info.Data().As<String>(), Key(isolate, ":")), text));
      } else info.GetReturnValue().Set(result);
    }
    return;
  }
  auto result = Object::New(isolate);
  if (!result->SetPrototypeV2(context, Null(isolate)).FromMaybe(false)) return;
  bool okay = Put(context, result, "owned", Boolean::New(isolate, owned)) &&
      Put(context, result, "available", Boolean::New(isolate, available)) &&
      Put(context, result, "cleared", Boolean::New(isolate, cleared)) &&
      Put(context, result, "metadataValid", Boolean::New(isolate, metadata_valid)) &&
      Put(context, result, "excludedOwner", Integer::New(isolate, excluded_owner)) &&
      Put(context, result, "failure", Integer::New(isolate, failure)) &&
      Put(context, result, "rejection", Integer::New(isolate, rejection));
  if (owned && metadata_valid) {
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

void Capture(const FunctionCallbackInfo<Value>& info) { CaptureImpl(info, false); }
void CaptureOrdinary(const FunctionCallbackInfo<Value>& info) { CaptureImpl(info, true); }
void CapturePacked(const FunctionCallbackInfo<Value>& info) { CaptureImpl(info, false, true); }
void CaptureOrdinaryPacked(const FunctionCallbackInfo<Value>& info) { CaptureImpl(info, true, true); }
void CaptureString(const FunctionCallbackInfo<Value>& info) { CaptureImpl(info, false, false, true); }
void CaptureOrdinaryString(const FunctionCallbackInfo<Value>& info) { CaptureImpl(info, true, false, true); }

void IsInspectableOwner(const FunctionCallbackInfo<Value>& info) {
  info.GetReturnValue().Set(info.Length() == 1 && OrdinaryForInspection(info[0]));
}

void CreateContextCapture(const FunctionCallbackInfo<Value>& info);
void CreatePackedContextCapture(const FunctionCallbackInfo<Value>& info);
void CreateStringContextCapture(const FunctionCallbackInfo<Value>& info);

void CreateCaptureImpl(const FunctionCallbackInfo<Value>& info, bool packed, bool string_summary = false) {
  auto isolate = info.GetIsolate();
  auto context = isolate->GetCurrentContext();
  if (info.Length() < 1 || info.Length() > 2 ||
      (info.Length() == 2 && !info[1]->IsBoolean()) || !info[0]->IsString() ||
      info[0].As<String>()->Length() != 64) {
    isolate->ThrowException(Exception::TypeError(Key(isolate, "invalid nonce")));
    return;
  }
  Local<Function> capture, context_factory;
  bool ordinary_owners = info.Length() == 2 && info[1]->IsTrue();
  auto callback = string_summary ? (ordinary_owners ? CaptureOrdinaryString : CaptureString)
                        : packed ? (ordinary_owners ? CaptureOrdinaryPacked : CapturePacked)
                         : (ordinary_owners ? CaptureOrdinary : Capture);
  if (Function::New(context, callback, info[0], 1, ConstructorBehavior::kThrow)
          .ToLocal(&capture) &&
      Function::New(context, string_summary ? CreateStringContextCapture :
                             packed ? CreatePackedContextCapture : CreateContextCapture, Local<Value>(), 2,
                     ConstructorBehavior::kThrow).ToLocal(&context_factory) &&
      Put(context, capture, "createContextCapture", context_factory))
    info.GetReturnValue().Set(capture);
}
void CreateCapture(const FunctionCallbackInfo<Value>& info) { CreateCaptureImpl(info, false); }
void CreatePackedCapture(const FunctionCallbackInfo<Value>& info) { CreateCaptureImpl(info, true); }
void CreateStringCapture(const FunctionCallbackInfo<Value>& info) { CreateCaptureImpl(info, false, true); }

// A foreign Inspector handle cannot be an argument to our root-context
// function. Find a fresh, private holder by an unguessable own property name,
// then install the same native callback in the holder's creation context.
// The heap iterator prohibits GC: the predicate checks only property presence,
// never values/descriptors, and allocates no JS objects. Full validation and
// function allocation happen after the iterator is gone. Retain at most two
// candidates so duplicate holders fail closed rather than choosing one.
class HolderPredicate final : public QueryObjectPredicate {
 public:
  HolderPredicate(Local<Context> context, Local<Name> key)
      : context_(context), key_(key) {}
  bool Filter(Local<Object> object) override {
    if (matches_ >= 2 || !OrdinaryForInspection(object) ||
        !object->GetPrototypeV2()->IsNull()) return false;
    auto present = object->HasRealNamedProperty(context_, key_);
    if (present.IsNothing()) { failed_ = true; return false; }
    if (!present.FromJust()) return false;
    ++matches_;
    return true;
  }
  bool failed() const { return failed_; }
 private:
  Local<Context> context_;
  Local<Name> key_;
  unsigned matches_ = 0;
  bool failed_ = false;
};

void CreateContextCaptureImpl(const FunctionCallbackInfo<Value>& info, bool packed, bool string_summary = false) {
  auto isolate = info.GetIsolate();
  HandleScope scope(isolate);
  auto context = isolate->GetCurrentContext();
  TryCatch caught(isolate);
  bool okay = false;
  do {
    if (info.Length() < 2 || info.Length() > 3 ||
        (info.Length() == 3 && !info[2]->IsBoolean()) || !info[0]->IsString() ||
        info[0].As<String>()->Length() != 64 || !info[1]->IsString() ||
        info[1].As<String>()->Length() != 64) break;
    // Intern before heap iteration: constructing a PropertyKey from an
    // arbitrary string could otherwise allocate while the iterator is live.
    String::Utf8Value key_bytes(isolate, info[1]);
    if (!*key_bytes || key_bytes.length() != 64) break;
    bool hex = true;
    for (int index = 0; index < 64; ++index) {
      char c = (*key_bytes)[index];
      if (!((c >= '0' && c <= '9') || (c >= 'a' && c <= 'f'))) hex = false;
    }
    if (!hex) break;
    Local<String> key;
    if (!String::NewFromUtf8(isolate, *key_bytes, NewStringType::kInternalized,
                            64).ToLocal(&key)) break;
    HolderPredicate predicate(context, key);
    std::vector<Global<Object>> holders;
    isolate->GetHeapProfiler()->QueryObjects(context, &predicate, &holders);
    if (predicate.failed() || holders.size() != 1) break;
    auto holder = holders[0].Get(isolate);
    Local<Value> marker, old_capture;
    Local<Array> names;
    if (!holder->GetOwnPropertyNames(context).ToLocal(&names) ||
        names->Length() != 2 ||
        !OwnDataKey(context, holder, key, &marker, true) || !marker->IsTrue() ||
        !OwnData(context, holder, "capture", &old_capture, true) ||
        !old_capture->IsUndefined()) break;
    Local<Context> destination;
    if (!holder->GetCreationContext().ToLocal(&destination)) break;
    Context::Scope destination_scope(destination);
    Local<Function> capture;
    bool ordinary_owners = info.Length() == 3 && info[2]->IsTrue();
    auto callback = string_summary ? (ordinary_owners ? CaptureOrdinaryString : CaptureString)
                           : packed ? (ordinary_owners ? CaptureOrdinaryPacked : CapturePacked)
                           : (ordinary_owners ? CaptureOrdinary : Capture);
    if (!Function::New(destination, callback, info[0], 1,
                       ConstructorBehavior::kThrow).ToLocal(&capture)) break;
    okay = holder->Set(destination, key, Null(isolate)).FromMaybe(false) &&
        holder->Set(destination, Key(isolate, "capture"), capture).FromMaybe(false);
  } while (false);
  if (caught.HasCaught()) { caught.Reset(); okay = false; }
  info.GetReturnValue().Set(Boolean::New(isolate, okay));
}
void CreateContextCapture(const FunctionCallbackInfo<Value>& info) { CreateContextCaptureImpl(info, false); }
void CreatePackedContextCapture(const FunctionCallbackInfo<Value>& info) { CreateContextCaptureImpl(info, true); }
void CreateStringContextCapture(const FunctionCallbackInfo<Value>& info) { CreateContextCaptureImpl(info, false, true); }

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
#ifdef QUOD_NATIVE_INSPECTOR_EXPERIMENT
#include "native-inspector-capture-experiment.inc"
#endif

void Initialize(Local<Object> exports, Local<Value>, Local<Context> context,
                void*) {
  Local<Function> factory;
  if (Function::New(context, CreateCapture).ToLocal(&factory))
    (void)exports->CreateDataProperty(context, Key(context->GetIsolate(),
                                                 "createCapture"), factory)
        .FromMaybe(false);
  if (Function::New(context, CreatePackedCapture).ToLocal(&factory))
    (void)exports->CreateDataProperty(context, Key(context->GetIsolate(),
                                                 "createPackedCapture"), factory)
        .FromMaybe(false);
  if (Function::New(context, CreateStringCapture).ToLocal(&factory))
    (void)exports->CreateDataProperty(context, Key(context->GetIsolate(),
                                                 "createStringCapture"), factory)
        .FromMaybe(false);
  if (Function::New(context, CreateContextCapture).ToLocal(&factory))
    (void)exports->CreateDataProperty(context, Key(context->GetIsolate(),
                                                 "createContextCapture"), factory)
        .FromMaybe(false);
  if (Function::New(context, IsInspectableOwner).ToLocal(&factory))
    (void)exports->CreateDataProperty(context, Key(context->GetIsolate(),
                                                 "isInspectableOwner"), factory)
        .FromMaybe(false);
  if (Function::New(context, CreateNativeDataFixture).ToLocal(&factory))
    (void)exports->CreateDataProperty(context, Key(context->GetIsolate(),
                                                 "createNativeDataFixture"), factory)
        .FromMaybe(false);
#ifdef QUOD_NATIVE_INSPECTOR_EXPERIMENT
  if (Function::New(context, CreateInspectorCapture).ToLocal(&factory))
    (void)exports->CreateDataProperty(context, Key(context->GetIsolate(),
                                                 "createInspectorCapture"), factory)
        .FromMaybe(false);
#endif
}
}  // namespace

NODE_MODULE_CONTEXT_AWARE(NODE_GYP_MODULE_NAME, Initialize)

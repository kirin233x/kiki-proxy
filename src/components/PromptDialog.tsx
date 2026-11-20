import { useState } from "react";
import { KeyRound } from "lucide-react";
import type { PromptForm } from "../api";
import { Field, Modal } from "./ui";

/** 插件请求的登录表单（账号密码、验证码等） */
export default function PromptDialog(props: {
  form: PromptForm;
  onSubmit: (values: Record<string, string>) => void;
  onCancel: () => void;
}) {
  const { form } = props;
  const [values, setValues] = useState<Record<string, string>>(() => Object.fromEntries(form.fields.map((f) => [f.key, f.default ?? ""])));
  const submit = () => props.onSubmit(values);
  return (
    <Modal
      title={form.title}
      icon={<KeyRound size={18} />}
      onClose={props.onCancel}
      footer={
        <>
          <button className="btn" onClick={props.onCancel}>
            取消
          </button>
          <button className="btn btn-primary" onClick={submit}>
            {form.submitText || "确定"}
          </button>
        </>
      }
    >
      {form.message && <p className="prompt-msg">{form.message}</p>}
      {form.image && <img className="prompt-img" src={form.image} alt="" />}
      <form
        onSubmit={(e) => {
          e.preventDefault();
          submit();
        }}
      >
        {form.fields.map((f, i) => (
          <Field key={f.key} label={f.label}>
            <input
              className={`input ${f.kind === "code" ? "input-code" : ""}`}
              type={f.kind === "password" ? "password" : "text"}
              inputMode={f.kind === "code" ? "numeric" : undefined}
              autoComplete={f.kind === "code" ? "one-time-code" : "off"}
              autoFocus={i === form.fields.findIndex((x) => !x.default)}
              placeholder={f.placeholder}
              value={values[f.key] ?? ""}
              onChange={(e) => setValues({ ...values, [f.key]: e.target.value })}
            />
          </Field>
        ))}
        <button type="submit" hidden />
      </form>
    </Modal>
  );
}

import { useEffect, useMemo, useRef, useState } from "react";
import { CloudDownload, Search } from "lucide-react";
import { api, applyModelInfo, EndpointConfig, fmtContext, ModelConfig, ModelInfo, newModel } from "../api";
import { Modal } from "./ui";

/** 从上游的 /models 接口拉模型列表，勾选后加入端点（顺便从价格表填好价格和上下文） */
export default function UpstreamModelsDialog(props: {
  endpoint: EndpointConfig;
  beforeFetch: () => Promise<unknown>;
  onClose: () => void;
  onAdd: (models: ModelConfig[]) => void;
}) {
  const ep = props.endpoint;
  const [ids, setIds] = useState<string[] | null>(null);
  const [infos, setInfos] = useState<Record<string, ModelInfo | null>>({});
  const [error, setError] = useState("");
  const [q, setQ] = useState("");
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const existing = useMemo(() => new Set(ep.models.flatMap((m) => [m.name, m.upstream].filter(Boolean))), [ep.models]);

  const started = useRef(false);
  useEffect(() => {
    if (started.current) return; // StrictMode 下只请求一次
    started.current = true;
    (async () => {
      try {
        await props.beforeFetch();
        const list = await api.listUpstreamModels(ep.id);
        setIds(list);
        // 价格表查不到不影响添加
        api
          .lookupModels(list, ep.baseUrl, ep.protocol)
          .then((r) => setInfos(Object.fromEntries(list.map((id, i) => [id, r[i]]))))
          .catch(() => {});
      } catch (e) {
        setError(String(e));
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const shown = (ids ?? []).filter((id) => id.toLowerCase().includes(q.toLowerCase()));
  const toggle = (id: string) => {
    const s = new Set(picked);
    if (s.has(id)) s.delete(id);
    else s.add(id);
    setPicked(s);
  };

  const add = () => {
    const taken = new Set(ep.models.map((m) => m.name));
    const models = [...picked].map((id) => {
      // 网关里的名字去掉日期后缀，更好记：claude-sonnet-4-5-20250929 → claude-sonnet-4-5
      const short = id.replace(/-\d{8}$/, "");
      const name = short !== id && !taken.has(short) ? short : id;
      taken.add(name);
      const m: ModelConfig = { ...newModel(), name, upstream: name === id ? "" : id };
      const info = infos[id];
      return info ? applyModelInfo(m, info) : m;
    });
    props.onAdd(models);
  };

  return (
    <Modal
      title="从上游获取模型"
      icon={<CloudDownload size={18} />}
      wide
      onClose={props.onClose}
      footer={
        <>
          <span className="muted-sm" style={{ marginRight: "auto" }}>
            {ids ? `共 ${ids.length} 个模型` : ""}
          </span>
          <button className="btn" onClick={props.onClose}>
            取消
          </button>
          <button className="btn btn-primary" disabled={!picked.size} onClick={add}>
            添加选中的 {picked.size || ""} 个
          </button>
        </>
      }
    >
      {error ? (
        <div className="banner err">{error}</div>
      ) : !ids ? (
        <div className="muted-sm" style={{ padding: "30px 0", textAlign: "center" }}>
          正在请求 {ep.name} 的模型列表…
        </div>
      ) : (
        <>
          <div className="search-box" style={{ marginBottom: 10 }}>
            <Search size={16} />
            <input className="input" placeholder="搜索" value={q} autoFocus onChange={(e) => setQ(e.target.value)} />
          </div>
          <div className="upstream-list">
            {shown.map((id) => {
              const added = existing.has(id);
              const info = infos[id];
              return (
                <label key={id} className={`upstream-row ${added ? "added" : ""}`}>
                  <input type="checkbox" disabled={added} checked={added || picked.has(id)} onChange={() => toggle(id)} />
                  <code>{id}</code>
                  {added ? (
                    <span className="muted-sm">已添加</span>
                  ) : info ? (
                    <span className="upstream-info">
                      <span>{fmtContext(info.contextWindow)}</span>
                      <span>
                        ${info.input} / ${info.output}
                      </span>
                      {info.vision && <span>图片</span>}
                    </span>
                  ) : (
                    <span className="muted-sm">{Object.keys(infos).length ? "价格表里没有" : ""}</span>
                  )}
                </label>
              );
            })}
            {shown.length === 0 && <div className="muted-sm" style={{ padding: 16 }}>没有匹配的模型</div>}
          </div>
        </>
      )}
    </Modal>
  );
}

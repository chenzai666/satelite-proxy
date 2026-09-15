import React, { useCallback, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { useRulesSidebarHeight } from "../../src/hooks/useRulesSidebarHeight";
import { useRulesetDragSort } from "../../src/hooks/useRulesetDragSort";
import { RulesetMenu } from "../../src/components/RulesetMenu";
import "../../src/App.css";

const zoom = Number(new URLSearchParams(location.search).get("zoom")) || 1;
document.documentElement.style.zoom = String(zoom);
document.documentElement.style.transition = "none";
function Fixture() {
  const [items, setItems] = useState(Array.from({length: 28}, (_, i) => ({ id: String(i + 1) })));
  const [menu, setMenu] = useState<string | null>(null);
  const anchor = useRef<HTMLElement | null>(null);
  const sidebar = useRulesSidebarHeight();
  const closeMenu = useCallback(() => setMenu(null), []);
  const { drag, onItemPointerDown } = useRulesetDragSort({ items, onReorder: setItems });
  const cards = items.filter(item => item.id !== drag?.id).map(item => (
    <div key={item.id} className="ruleset-item" data-ruleset-id={item.id}
      style={{ minHeight: 65 }} onPointerDown={event => onItemPointerDown(item.id, event)}>
      <span>规则集 {item.id}</span>
      <button data-ruleset-menu data-menu={item.id} onClick={event => { anchor.current = event.currentTarget; setMenu(item.id); }}>⋮</button>
    </div>
  ));
  if (drag) cards.splice(drag.insertIndex, 0, <div key="gap" style={{height: drag.height}} />);
  return <main className="main" style={{ height: `${window.innerHeight / zoom}px`, padding: 24, boxSizing: "border-box", overflow: "auto" }}>
    <header style={{height: 155}}>设置 · 规则</header>
    <div className="rules-layout">
      <aside ref={sidebar} className="card ruleset-list rules-route-list">
        <div className="ruleset-list-actions"><button>新建</button><button>重置</button></div>
        <div className="ruleset-list-title">规则集 · 拖拽排序</div>
        <div className="ruleset-scroll" data-ruleset-scroll tabIndex={0}>{cards}</div>
      </aside>
      <section className="rules-main"><div className="card" style={{height: 950}}>右侧内容保持位置</div></section>
    </div>
    {menu && <RulesetMenu anchor={anchor.current} onClose={closeMenu}>
      <div className="rule-menu-subhost"><button className="rule-menu-item">路由子菜单</button>
        <div className="rule-menu-sub"><button className="rule-menu-item" data-sub-action onClick={closeMenu}>直连</button></div>
      </div>
      <button className="rule-menu-item" onClick={closeMenu}>编辑</button>
      <button className="rule-menu-item" onClick={closeMenu}>删除</button>
    </RulesetMenu>}
    <output id="order" hidden>{items.map(item => item.id).join(",")}</output>
  </main>;
}
createRoot(document.getElementById("root")!).render(<Fixture />);

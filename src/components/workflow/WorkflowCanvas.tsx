// React Flow canvas for the workflow editor (P27). Controlled by the editor
// page; renders skill steps as draggable, connectable nodes. Edges express
// dependencies (target depends on source).

import {
  Background,
  Controls,
  ReactFlow,
  useReactFlow,
  type Connection,
  type Edge,
  type NodeProps,
  type OnEdgesChange,
  type OnNodesChange,
} from "@xyflow/react";
import "@xyflow/react/dist/style.css";
import type { StepNode } from "@/lib/workflow";

function StepNodeView({ id, data, selected }: NodeProps<StepNode>) {
  const { setNodes } = useReactFlow();
  return (
    <div
      className={cn(
        "min-w-[140px] rounded border bg-card px-2.5 py-2 text-card-foreground",
        selected ? "border-accent ring-1 ring-ring" : "border-border",
      )}
    >
      <div className="mb-1 text-[10px] uppercase tracking-wide text-muted-foreground">
        步骤 · {id}
      </div>
      <input
        className="w-full select-text rounded-sm border border-input bg-background px-1.5 py-1 font-mono text-[12px] text-foreground outline-none focus-visible:ring-1 focus-visible:ring-ring"
        value={data.skill}
        spellCheck={false}
        onChange={(e) =>
          setNodes((nds) =>
            nds.map((n) =>
              n.id === id ? { ...n, data: { ...n.data, skill: e.target.value } } : n,
            ),
          )
        }
      />
    </div>
  );
}

const nodeTypes = { step: StepNodeView };

function cn(...classes: Array<string | false | undefined>): string {
  return classes.filter(Boolean).join(" ");
}

interface WorkflowCanvasProps {
  nodes: StepNode[];
  edges: Edge[];
  onNodesChange: OnNodesChange<StepNode>;
  onEdgesChange: OnEdgesChange;
  onConnect: (c: Connection) => void;
}

export function WorkflowCanvas({
  nodes,
  edges,
  onNodesChange,
  onEdgesChange,
  onConnect,
}: WorkflowCanvasProps) {
  return (
    <div className="h-full w-full">
      <ReactFlow
        nodes={nodes}
        edges={edges}
        nodeTypes={nodeTypes}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onConnect={onConnect}
        fitView
        deleteKeyCode={["Backspace", "Delete"]}
        proOptions={{ hideAttribution: true }}
        className="bg-background"
        style={{ height: "100%" }}
      >
        <Background color="var(--border)" gap={16} />
        <Controls showInteractive={false} />
      </ReactFlow>
    </div>
  );
}

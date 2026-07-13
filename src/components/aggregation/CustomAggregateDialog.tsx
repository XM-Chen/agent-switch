/**
 * 自定义聚合创建/编辑对话框（C4，R4）
 *
 * 命名 + 从现有自动聚合中多选成员 + dnd-kit 拖拽排序。成员存的是自动聚合 key
 * （模型 id 原文）。保存走 create/update 命令。
 */

import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  DndContext,
  closestCenter,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  arrayMove,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { GripVertical, X, Plus, Loader2 } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Badge } from "@/components/ui/badge";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { toast } from "sonner";
import type { AppId } from "@/lib/api";
import type { AggregateView, CustomAggregateView } from "@/lib/api/aggregation";
import {
  useCreateCustomAggregate,
  useUpdateCustomAggregate,
} from "@/lib/query/aggregation";

interface CustomAggregateDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  appId: AppId;
  aggregates: AggregateView[];
  /** 传入时为编辑模式；不传为新建。 */
  editing?: CustomAggregateView | null;
  /**
   * 编辑模式下的初始成员 key（自动聚合 key）。
   *
   * C2 的 `CustomAggregateView` 只回传展平候选，不含原始 ordered_members，
   * 故由父组件用 `reconstructMemberKeys` 近似重建后传入。
   */
  editingMemberKeys?: string[];
}

export function CustomAggregateDialog({
  open,
  onOpenChange,
  appId,
  aggregates,
  editing,
  editingMemberKeys,
}: CustomAggregateDialogProps) {
  const { t } = useTranslation();
  const [name, setName] = useState("");
  const [members, setMembers] = useState<string[]>([]);
  const [pendingMember, setPendingMember] = useState("");

  const createMutation = useCreateCustomAggregate();
  const updateMutation = useUpdateCustomAggregate();
  const isSaving = createMutation.isPending || updateMutation.isPending;

  // 打开时初始化草稿：编辑模式用父组件重建的成员 key，新建模式清空。
  useEffect(() => {
    if (open) {
      setName(editing?.name ?? "");
      setMembers(editingMemberKeys ?? []);
      setPendingMember("");
    }
  }, [open, editing, editingMemberKeys]);

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 8 } }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  );

  // 可添加的自动聚合 key（排除已选）。
  const availableKeys = useMemo(
    () => aggregates.map((a) => a.key).filter((k) => !members.includes(k)),
    [aggregates, members],
  );

  const handleDragEnd = (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) return;
    const oldIndex = members.indexOf(String(active.id));
    const newIndex = members.indexOf(String(over.id));
    if (oldIndex === -1 || newIndex === -1) return;
    setMembers((prev) => arrayMove(prev, oldIndex, newIndex));
  };

  const handleAddMember = () => {
    if (!pendingMember) return;
    if (members.includes(pendingMember)) return;
    setMembers((prev) => [...prev, pendingMember]);
    setPendingMember("");
  };

  const handleRemoveMember = (key: string) => {
    setMembers((prev) => prev.filter((m) => m !== key));
  };

  const handleSave = () => {
    const trimmed = name.trim();
    if (!trimmed) {
      toast.error(
        t("aggregation.custom.nameRequired", {
          defaultValue: "请输入聚合名称",
        }),
      );
      return;
    }
    const onDone = () => onOpenChange(false);
    if (editing) {
      updateMutation.mutate(
        { appType: appId, id: editing.id, name: trimmed, members },
        { onSuccess: onDone },
      );
    } else {
      createMutation.mutate(
        { appType: appId, name: trimmed, members },
        { onSuccess: onDone },
      );
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent zIndex="nested" className="max-w-lg">
        <DialogHeader>
          <DialogTitle>
            {editing
              ? t("aggregation.custom.editTitle", {
                  defaultValue: "编辑自定义聚合",
                })
              : t("aggregation.custom.createTitle", {
                  defaultValue: "新建自定义聚合",
                })}
          </DialogTitle>
          <DialogDescription>
            {t("aggregation.custom.dialogDescription", {
              defaultValue:
                "从现有自动聚合中选择成员并排序，路由将按成员顺序依次尝试。",
            })}
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4 overflow-y-auto px-6 py-4">
          <div className="space-y-2">
            <Label htmlFor="custom-aggregate-name">
              {t("aggregation.custom.nameLabel", { defaultValue: "名称" })}
            </Label>
            <Input
              id="custom-aggregate-name"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t("aggregation.custom.namePlaceholder", {
                defaultValue: "例如：主力聚合",
              })}
            />
          </div>

          <div className="space-y-2">
            <Label>
              {t("aggregation.custom.membersLabel", {
                defaultValue: "成员（自动聚合）",
              })}
            </Label>
            <div className="flex gap-2">
              <Select
                value={pendingMember}
                onValueChange={setPendingMember}
                disabled={availableKeys.length === 0}
              >
                <SelectTrigger className="flex-1">
                  <SelectValue
                    placeholder={
                      availableKeys.length === 0
                        ? t("aggregation.custom.noMoreMembers", {
                            defaultValue: "无更多可选聚合",
                          })
                        : t("aggregation.custom.selectMember", {
                            defaultValue: "选择要加入的自动聚合",
                          })
                    }
                  />
                </SelectTrigger>
                <SelectContent>
                  {availableKeys.map((key) => (
                    <SelectItem key={key} value={key}>
                      {key}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <Button
                variant="outline"
                size="sm"
                onClick={handleAddMember}
                disabled={!pendingMember}
              >
                <Plus className="h-4 w-4" />
                {t("aggregation.custom.addMember", { defaultValue: "加入" })}
              </Button>
            </div>

            {members.length === 0 ? (
              <p className="text-sm text-muted-foreground">
                {t("aggregation.custom.noMembers", {
                  defaultValue: "尚未添加成员。",
                })}
              </p>
            ) : (
              <DndContext
                sensors={sensors}
                collisionDetection={closestCenter}
                onDragEnd={handleDragEnd}
              >
                <SortableContext
                  items={members}
                  strategy={verticalListSortingStrategy}
                >
                  <ul className="space-y-1.5">
                    {members.map((key, index) => (
                      <SortableMemberRow
                        key={key}
                        memberKey={key}
                        priority={index + 1}
                        onRemove={() => handleRemoveMember(key)}
                      />
                    ))}
                  </ul>
                </SortableContext>
              </DndContext>
            )}
          </div>
        </div>

        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            {t("common.cancel", { defaultValue: "取消" })}
          </Button>
          <Button onClick={handleSave} disabled={isSaving}>
            {isSaving && <Loader2 className="h-4 w-4 animate-spin" />}
            {t("common.save", { defaultValue: "保存" })}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

interface SortableMemberRowProps {
  memberKey: string;
  priority: number;
  onRemove: () => void;
}

function SortableMemberRow({
  memberKey,
  priority,
  onRemove,
}: SortableMemberRowProps) {
  const {
    setNodeRef,
    attributes,
    listeners,
    transform,
    transition,
    isDragging,
  } = useSortable({ id: memberKey });

  return (
    <li
      ref={setNodeRef}
      style={{
        transform: CSS.Transform.toString(transform),
        transition,
        opacity: isDragging ? 0.6 : 1,
      }}
      className="flex items-center gap-2 rounded-md border border-border bg-card/40 px-2 py-1.5 text-sm"
    >
      <button
        type="button"
        className="cursor-grab text-muted-foreground active:cursor-grabbing"
        {...attributes}
        {...listeners}
      >
        <GripVertical className="h-4 w-4" />
      </button>
      <Badge variant="outline" className="shrink-0">
        P{priority}
      </Badge>
      <span className="min-w-0 flex-1 truncate font-mono text-xs">
        {memberKey}
      </span>
      <button
        type="button"
        className="text-muted-foreground hover:text-destructive"
        onClick={onRemove}
      >
        <X className="h-4 w-4" />
      </button>
    </li>
  );
}

"use client"

import { invoke } from "@tauri-apps/api/core"
import { Volume2 } from "lucide-react"
import { useCallback, useEffect, useState } from "react"
import { toast } from "sonner"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "./ui/select"
import { Switch } from "./ui/switch"

interface TtsModel {
  id: string
  label: string
  installed: boolean
}

interface TtsStatus {
  enabled: boolean
  model: string
  models: TtsModel[]
  problem: string | null
}

/**
 * Turn reading summaries aloud on or off, and pick which Qwen3-TTS checkpoint
 * reads them. Models are installed by hand for now, so a checkpoint that is not
 * on disk is shown but cannot be selected.
 */
export function SummarySpeechSettings() {
  const [status, setStatus] = useState<TtsStatus | null>(null)
  const [isSaving, setIsSaving] = useState(false)

  useEffect(() => {
    void invoke<TtsStatus>("tts_get_status")
      .then(setStatus)
      .catch((error) => console.error("Could not read TTS status:", error))
  }, [])

  const save = useCallback(
    async (next: { enabled: boolean; model: string }) => {
      setIsSaving(true)
      try {
        await invoke("tts_set_settings", { settings: next })
        setStatus((current) => (current ? { ...current, ...next } : current))
      } catch (error) {
        console.error("Could not save TTS settings:", error)
        toast.error("Could not save the speech settings")
      } finally {
        setIsSaving(false)
      }
    },
    []
  )

  if (!status) return null

  const installed = status.models.filter((model) => model.installed)
  const isInstalled = installed.length > 0 && status.problem === null

  return (
    <div className="bg-white rounded-lg border border-gray-200 p-6 shadow-sm">
      <div className="flex items-center justify-between">
        <div className="flex-1">
          <div className="flex items-center gap-2 mb-2">
            <Volume2 className="h-5 w-5 text-gray-600" />
            <h3 className="text-lg font-semibold text-gray-900">
              Read summaries aloud
            </h3>
            <span className="px-2 py-0.5 text-xs font-medium bg-yellow-100 text-yellow-800 rounded-full">
              BETA
            </span>
          </div>
          <p className="text-sm text-gray-600">
            Adds a play button next to the summary. Speech is synthesized on this
            machine, offline.
          </p>
          <p className="mt-2 text-xs text-gray-500">
            {isInstalled
              ? "Runs on the GPU through llama.cpp."
              : status.problem ?? "No speech model is installed."}
          </p>
        </div>

        <div className="ml-6">
          <Switch
            checked={status.enabled && isInstalled}
            disabled={!isInstalled || isSaving}
            onCheckedChange={(checked) =>
              void save({ enabled: checked, model: status.model })
            }
          />
        </div>
      </div>

      {status.models.length > 0 && (
        <div className="mt-4 flex items-center justify-between gap-4 border-t border-gray-100 pt-4">
          <div>
            <p className="text-sm font-medium text-gray-900">Model</p>
            <p className="text-xs text-gray-500">
              Applies to the next summary you play.
            </p>
          </div>
          <Select
            value={status.model}
            disabled={!status.enabled || isSaving}
            onValueChange={(value) =>
              void save({ enabled: status.enabled, model: value })
            }
          >
            <SelectTrigger className="w-60">
              <SelectValue placeholder="Select a model" />
            </SelectTrigger>
            <SelectContent>
              {status.models.map((model) => (
                <SelectItem
                  key={model.id}
                  value={model.id}
                  disabled={!model.installed}
                >
                  {model.label}
                  {model.installed ? "" : " — not installed"}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      )}
    </div>
  )
}

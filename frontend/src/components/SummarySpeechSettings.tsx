"use client"

import { invoke } from "@tauri-apps/api/core"
import { listen } from "@tauri-apps/api/event"
import { Download, Loader2, Volume2 } from "lucide-react"
import { Button } from "./ui/button"
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
import { translate, useI18n } from "@/lib/i18n"

interface TtsModel {
  id: string
  label: string
  installed: boolean
  downloadable: boolean
}

interface TtsStatus {
  enabled: boolean
  model: string
  autoPrepare: boolean
  models: TtsModel[]
  problem: string | null
}

/**
 * Turn reading summaries aloud on or off, and pick which Qwen3-TTS checkpoint
 * reads them. A checkpoint that is not on disk cannot be selected; the ones the
 * app can fetch get a Download button.
 */
export function SummarySpeechSettings() {
  const { t } = useI18n()
  const [status, setStatus] = useState<TtsStatus | null>(null)
  const [isSaving, setIsSaving] = useState(false)
  const [download, setDownload] = useState<{ model: string; progress: number } | null>(null)

  const refresh = useCallback(() => {
    void invoke<TtsStatus>("tts_get_status")
      .then(setStatus)
      .catch((error) => console.error("Could not read TTS status:", error))
  }, [])

  useEffect(() => {
    refresh()
    const unlisten = listen<{ model: string; progress: number }>("tts-download-progress", ({ payload }) => {
      setDownload(payload.progress < 100 ? payload : null)
    })
    return () => { void unlisten.then((stop) => stop()) }
  }, [refresh])

  const downloadModel = async (model: string) => {
    setDownload({ model, progress: 0 })
    try {
      await invoke("tts_download_model", { model })
      toast.success(translate("The speech model is ready"))
    } catch (error) {
      toast.error(translate("Could not download the speech model"), { description: String(error) })
    } finally {
      setDownload(null)
      refresh()
    }
  }

  const save = useCallback(
    async (next: { enabled: boolean; model: string; autoPrepare: boolean }) => {
      setIsSaving(true)
      try {
        await invoke("tts_set_settings", { settings: next })
        setStatus((current) => (current ? { ...current, ...next } : current))
      } catch (error) {
        console.error("Could not save TTS settings:", error)
        toast.error(translate("Could not save the speech settings"))
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
    <div className="bg-white rounded-lg border border-slate-200 p-6 shadow-sm">
      <div className="flex items-center justify-between">
        <div className="flex-1">
          <div className="flex items-center gap-2 mb-2">
            <Volume2 className="h-5 w-5 text-slate-600" />
            <h3 className="text-lg font-semibold text-slate-900">
              {t("Read summaries aloud")}
            </h3>
            <span className="px-2 py-0.5 text-xs font-medium bg-yellow-100 text-yellow-800 rounded-full">
              {t("BETA")}
            </span>
          </div>
          <p className="text-sm text-slate-600">
            {t("Adds a play button next to the summary. Speech is synthesized on this machine, offline.")}
          </p>
          <p className="mt-2 text-xs text-slate-500">
            {isInstalled
              ? t("Runs on the GPU through llama.cpp.")
              : status.problem ?? t("No speech model is installed.")}
          </p>
        </div>

        <div className="ml-6">
          <Switch
            checked={status.enabled && isInstalled}
            disabled={!isInstalled || isSaving}
            onCheckedChange={(checked) =>
              void save({ enabled: checked, model: status.model, autoPrepare: status.autoPrepare })
            }
          />
        </div>
      </div>

      {status.models.length > 0 && (
        <div className="mt-4 flex items-center justify-between gap-4 border-t border-slate-100 pt-4">
          <div>
            <p className="text-sm font-medium text-slate-900">{t("Model")}</p>
            <p className="text-xs text-slate-500">
              {t("Applies to the next summary you play.")}
            </p>
          </div>
          <Select
            value={status.model}
            disabled={!status.enabled || isSaving}
            onValueChange={(value) =>
              void save({ enabled: status.enabled, model: value, autoPrepare: status.autoPrepare })
            }
          >
            <SelectTrigger className="w-60">
              <SelectValue placeholder={t("Select a model")} />
            </SelectTrigger>
            <SelectContent>
              {status.models.map((model) => (
                <SelectItem
                  key={model.id}
                  value={model.id}
                  disabled={!model.installed}
                >
                  {model.label}
                  {model.installed ? "" : ` — ${t("not installed")}`}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
      )}

      {status.models.filter((model) => !model.installed && model.downloadable).map((model) => (
        <div key={model.id} className="mt-4 flex items-center justify-between gap-4 border-t border-slate-100 pt-4">
          <div>
            <p className="text-sm font-medium text-slate-900">{model.label}</p>
            <p className="text-xs text-slate-500">{t("~1.5 GB with the speech runtime. Downloaded from Hugging Face and the llama.cpp releases.")}</p>
          </div>
          {download?.model === model.id ? (
            <span className="flex shrink-0 items-center gap-1.5 text-sm font-medium text-indigo-600">
              <Loader2 className="h-4 w-4 animate-spin" /> {download.progress}%
            </span>
          ) : (
            <Button size="sm" disabled={download !== null} onClick={() => void downloadModel(model.id)}>
              <Download className="mr-1.5 h-4 w-4" /> {t("Download")}
            </Button>
          )}
        </div>
      ))}

      <div className="mt-4 flex items-center justify-between gap-4 border-t border-slate-100 pt-4">
        <div>
          <p className="text-sm font-medium text-slate-900">{t("Read new summaries right away")}</p>
          <p className="text-xs text-slate-500">
            {t("Each new or edited summary is read into a file in the background, so it plays at once. Off: only when you press Play.")}
          </p>
        </div>
        <Switch
          checked={status.autoPrepare}
          disabled={!status.enabled || !isInstalled || isSaving}
          onCheckedChange={(checked) =>
            void save({ enabled: status.enabled, model: status.model, autoPrepare: checked })
          }
        />
      </div>
    </div>
  )
}

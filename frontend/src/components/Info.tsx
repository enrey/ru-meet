import React from "react";
import { Info as InfoIcon } from "lucide-react";
import { Dialog, DialogContent, DialogTitle, DialogTrigger } from "./ui/dialog";
import { VisuallyHidden } from "./ui/visually-hidden";
import { About } from "./About";
import { useI18n } from "@/lib/i18n";

interface InfoProps {
    isCollapsed: boolean;
}

const Info = React.forwardRef<HTMLButtonElement, InfoProps>(({ isCollapsed }, ref) => {
  const { t } = useI18n();
  return (
    <Dialog aria-describedby={undefined}>
      <DialogTrigger asChild>
        <button 
          ref={ref} 
          className={`flex items-center justify-center mb-2 cursor-pointer border-none transition-colors ${
            isCollapsed 
              ? "bg-transparent p-2 hover:bg-slate-100 rounded-lg" 
              : "w-full px-3 py-1.5 mt-1 text-sm font-medium text-slate-700 bg-slate-200 hover:bg-slate-200 rounded-lg shadow-sm"
          }`}
          title={t('About Ru-Meet')}
        >
          <InfoIcon className={`text-slate-600 ${isCollapsed ? "w-5 h-5" : "w-4 h-4"}`} />
          {!isCollapsed && (
            <span className="ml-2 text-sm text-slate-700">{t('About')}</span>
          )}
        </button>
      </DialogTrigger>
      <DialogContent>
        <VisuallyHidden>
          <DialogTitle>{t('About Ru-Meet')}</DialogTitle>
        </VisuallyHidden>
        <About />
      </DialogContent>
    </Dialog>
  );
});

Info.displayName = "About";

export default Info; 
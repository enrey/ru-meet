import React from "react";
import Image from "next/image";
import { Dialog, DialogContent, DialogTitle, DialogTrigger } from "./ui/dialog";
import { VisuallyHidden } from "./ui/visually-hidden";
import { About } from "./About";
import { useI18n } from "@/lib/i18n";
import { useAppVersion } from "@/hooks/useAppVersion";

interface LogoProps {
  isCollapsed: boolean;
}

const Logo = React.forwardRef<HTMLButtonElement, LogoProps>(
  ({ isCollapsed }, ref) => {
    const { t } = useI18n();
    const appVersion = useAppVersion();
    return (
      <Dialog aria-describedby={undefined}>
        {isCollapsed ? (
          <DialogTrigger asChild>
            <button
              ref={ref}
              type="button"
              className="flex items-center justify-center mb-2 cursor-pointer bg-transparent border-none p-0 hover:opacity-80 transition-opacity"
              aria-label={t('About Ru-Meet')}
            >
              <Image
                src="/app-icon.png"
                alt="Ru-Meet"
                width={40}
                height={40}
                className="object-contain"
                priority
              />
            </button>
          </DialogTrigger>
        ) : (
          <DialogTrigger asChild>
            <button
              ref={ref}
              type="button"
              className="mb-2 flex w-full items-center gap-3 rounded-xl px-1 py-1 text-left cursor-pointer transition-colors hover:bg-slate-100 focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
              aria-label={t('About Ru-Meet')}
            >
              <Image
                src="/app-icon.png"
                alt=""
                width={40}
                height={40}
                className="shrink-0 object-contain"
                priority
              />
              <span className="flex items-baseline gap-2">
                <span className="text-lg font-semibold text-slate-900">Ru-Meet</span>
                {appVersion && <span className="text-xs text-slate-400">v{appVersion}</span>}
              </span>
            </button>
          </DialogTrigger>
        )}
        <DialogContent>
          <VisuallyHidden>
            <DialogTitle>{t('About Ru-Meet')}</DialogTitle>
          </VisuallyHidden>
          <About />
        </DialogContent>
      </Dialog>
    );
  },
);

Logo.displayName = "Logo";

export default Logo;

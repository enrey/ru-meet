"use client";

import { Button } from '@/components/ui/button';
import { ButtonGroup } from '@/components/ui/button-group';
import { Copy, Save, Loader2 } from 'lucide-react';
import { useI18n } from '@/lib/i18n';

interface SummaryUpdaterButtonGroupProps {
  isSaving: boolean;
  isDirty: boolean;
  onSave: () => Promise<void>;
  onCopy: () => Promise<void>;
}

export function SummaryUpdaterButtonGroup({
  isSaving,
  isDirty,
  onSave,
  onCopy,
}: SummaryUpdaterButtonGroupProps) {
  const { t } = useI18n();
  return (
    <ButtonGroup>
      {/* Save button */}
      <Button
        variant="outline"
        size="sm"
        className={`${isDirty ? 'bg-green-200' : ""}`}
        title={isSaving ? t('Saving') : t('Save Changes')}
        onClick={onSave}
        disabled={isSaving}
      >
        {isSaving ? (
          <>
            <Loader2 className="animate-spin" />
            <span className="hidden @[40rem]:inline">{t('Saving...')}</span>
          </>
        ) : (
          <>
            <Save />
            <span className="hidden @[40rem]:inline">{t('Save')}</span>
          </>
        )}
      </Button>

      {/* Copy button */}
      <Button
        variant="outline"
        size="sm"
        title={t('Copy Summary')}
        onClick={onCopy}
        className="cursor-pointer"
      >
        <Copy />
        <span className="hidden @[40rem]:inline">{t('Copy')}</span>
      </Button>

    </ButtonGroup>
  );
}

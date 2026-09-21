import React from 'react';

interface AudioLevelMeterProps {
  rmsLevel: number;    // 0.0 to 1.0
  peakLevel: number;   // 0.0 to 1.0
  isActive: boolean;   // Whether audio is being detected
  deviceName: string;
  className?: string;
  size?: 'small' | 'medium' | 'large';
}

const MIN_DBFS = -60;

const levelToDbfs = (level: number) => level > 0 ? 20 * Math.log10(level) : -Infinity;

const dbfsToMeterPercent = (dbfs: number) => {
  if (!Number.isFinite(dbfs)) return 0;
  return Math.max(0, Math.min(100, ((dbfs - MIN_DBFS) / -MIN_DBFS) * 100));
};

const formatDbfs = (dbfs: number) => {
  if (!Number.isFinite(dbfs)) return '−∞ dBFS';
  return `${Math.round(dbfs).toString().replace('-', '−')} dBFS`;
};

const getLevelColor = (dbfs: number) => {
  if (dbfs < -12) return 'bg-green-500';
  if (dbfs < -6) return 'bg-yellow-500';
  return 'bg-red-500';
};

export function AudioLevelMeter({
  rmsLevel,
  peakLevel,
  isActive,
  deviceName,
  className = '',
  size = 'medium'
}: AudioLevelMeterProps) {
  // Convert the linear PCM amplitude to the standard digital-audio scale.
  // The bar covers -60 dBFS (silence/noise floor) through 0 dBFS (clipping).
  const normalizedRms = Math.max(0, Math.min(1, rmsLevel));
  const normalizedPeak = Math.max(0, Math.min(1, peakLevel));
  const rmsDbfs = levelToDbfs(normalizedRms);
  const peakDbfs = levelToDbfs(normalizedPeak);
  const rmsPercent = dbfsToMeterPercent(rmsDbfs);
  const peakPercent = dbfsToMeterPercent(peakDbfs);
  const rmsColor = getLevelColor(rmsDbfs);
  const peakColor = getLevelColor(peakDbfs);

  // Size variants
  const sizeClasses = {
    small: {
      container: 'h-2',
      text: 'text-xs',
      meter: 'h-1.5'
    },
    medium: {
      container: 'h-3',
      text: 'text-sm',
      meter: 'h-2'
    },
    large: {
      container: 'h-4',
      text: 'text-base',
      meter: 'h-3'
    }
  };

  const sizes = sizeClasses[size];

  return (
    <div className={`flex items-center space-x-2 ${className}`}>
      {/* Device activity indicator */}
      <div className={`w-2 h-2 rounded-full ${
        isActive ? 'bg-green-400 animate-pulse' : 'bg-gray-300'
      }`} title={`${deviceName} - ${isActive ? 'Active' : 'Inactive'}`} />

      {/* Level meter container */}
      <div
        className={`flex-1 ${sizes.container} relative`}
        title={`RMS ${formatDbfs(rmsDbfs)}, peak ${formatDbfs(peakDbfs)}`}
      >
        {/* Background */}
        <div className="w-full h-full bg-gray-200 rounded-sm overflow-hidden">
          {/* RMS level bar (main level) */}
          <div
            className={`${sizes.meter} ${rmsColor} transition-all duration-150 ease-out rounded-sm`}
            style={{ width: `${rmsPercent}%` }}
          />

          {/* Peak level indicator (thin line) */}
          {peakPercent > rmsPercent && (
            <div
              className={`absolute top-0 bottom-0 w-0.5 ${peakColor} transition-all duration-75`}
              style={{ left: `${peakPercent}%` }}
            />
          )}
        </div>

        {/* Reference marks: -24, -12 and -6 dBFS. */}
        {[-24, -12, -6].map(dbfs => (
          <div
            key={dbfs}
            className="pointer-events-none absolute inset-y-0 w-px bg-gray-500 opacity-30"
            style={{ left: `${dbfsToMeterPercent(dbfs)}%` }}
          />
        ))}
      </div>

      {/* RMS value; 0 dBFS is the digital clipping ceiling. */}
      <div className={`${sizes.text} text-gray-600 font-mono min-w-[5.25rem] text-right`}>
        {formatDbfs(rmsDbfs)}
      </div>
    </div>
  );
}

interface CompactAudioLevelMeterProps {
  rmsLevel: number;
  peakLevel: number;
  isActive: boolean;
  className?: string;
}

// Compact version for inline display in dropdowns
export function CompactAudioLevelMeter({
  rmsLevel,
  peakLevel,
  isActive,
  className = ''
}: CompactAudioLevelMeterProps) {
  const normalizedRms = Math.max(0, Math.min(1, rmsLevel));
  const rmsDbfs = levelToDbfs(normalizedRms);
  const rmsPercent = dbfsToMeterPercent(rmsDbfs);

  return (
    <div className={`flex items-center space-x-1 ${className}`}>
      {/* Activity dot */}
      <div className={`w-1.5 h-1.5 rounded-full ${
        isActive ? 'bg-green-400' : 'bg-gray-300'
      }`} />

      {/* Mini meter */}
      <div className="w-8 h-1.5 bg-gray-200 rounded-sm overflow-hidden">
        <div
          className={`h-full ${getLevelColor(rmsDbfs)} transition-all duration-150`}
          style={{ width: `${rmsPercent}%` }}
        />
      </div>
    </div>
  );
}

import { AutoComplete, ConfigProvider, Input } from 'antd'
import { ChevronDown } from 'lucide-react'
import { isConcreteModel } from '../lib/model-rules'

export function ModelSelect({ id, name, value, onChange, models, invalid, disabled, required }: { id: string; name: string; value: string; onChange: (value: string) => void; models: string[]; invalid?: boolean; disabled?: boolean; required?: boolean }) {
  const options = [...new Set(models.map(model => model.trim()).filter(isConcreteModel))].map(model => ({ value: model }))
  return <ConfigProvider theme={{ token: { motion: false } }}><AutoComplete id={id} className="w-full" value={value} onChange={onChange} options={options} disabled={disabled} maxLength={256} status={invalid ? 'error' : undefined} filterOption={(input, option) => String(option?.value ?? '').toLowerCase().includes(input.toLowerCase())} getPopupContainer={trigger => trigger.parentElement ?? document.body}>
    <Input id={id} name={name} required={required} aria-invalid={invalid || undefined} autoComplete="off" spellCheck={false} placeholder="选择或输入具体模型名" suffix={<ChevronDown aria-hidden="true" className="size-3.5 text-muted-foreground" />} />
  </AutoComplete></ConfigProvider>
}

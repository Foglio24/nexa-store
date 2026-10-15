import {
  Clapperboard, Code2, Cpu, FlaskConical, Gamepad2, Globe, GraduationCap, Image, Briefcase, Wrench,
  type LucideIcon,
} from 'lucide-react';
import type { CategoryId } from '../types';

export interface CategoryInfo {
  id: CategoryId;
  /** English text; the interface translates it with `t()`. */
  label: string;
  description: string;
  icon: LucideIcon;
  /** Hue (0–360) used to tint the category's tile, banner and icon. */
  hue: number;
}

export const categories: CategoryInfo[] = [
  { id: 'games', label: 'Games', description: 'Action, strategy, puzzles and pastimes', icon: Gamepad2, hue: 340 },
  { id: 'internet', label: 'Internet', description: 'Browsers, messaging, mail and sharing', icon: Globe, hue: 212 },
  { id: 'audio-video', label: 'Audio & video', description: 'Music, films, recording and editing', icon: Clapperboard, hue: 270 },
  { id: 'graphics', label: 'Graphics', description: 'Drawing, photo editing, 3D and viewers', icon: Image, hue: 24 },
  { id: 'office', label: 'Office', description: 'Documents, spreadsheets, calendar and finance', icon: Briefcase, hue: 150 },
  { id: 'development', label: 'Development', description: 'Editors, IDEs and programming tools', icon: Code2, hue: 190 },
  { id: 'education', label: 'Education', description: 'Learn languages, subjects and more', icon: GraduationCap, hue: 45 },
  { id: 'science', label: 'Science', description: 'Maths, astronomy, electronics and data', icon: FlaskConical, hue: 300 },
  { id: 'utilities', label: 'Utilities', description: 'Everyday tools, files and terminal', icon: Wrench, hue: 100 },
  { id: 'system', label: 'System', description: 'Monitoring, security and settings', icon: Cpu, hue: 225 },
];

const byId = new Map(categories.map(category => [category.id, category]));
export const categoryInfo = (id: string): CategoryInfo | undefined => byId.get(id as CategoryId);

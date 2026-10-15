import {
  Clapperboard, Code2, Cpu, FlaskConical, Gamepad2, Globe, GraduationCap, Image, Briefcase, Wrench,
  type LucideIcon,
} from 'lucide-react';
import type { CategoryId } from '../types';

export interface CategoryInfo {
  id: CategoryId;
  label: string;
  description: string;
  icon: LucideIcon;
  /** Hue (0–360) used to tint the category's tile, banner and icon. */
  hue: number;
}

export const categories: CategoryInfo[] = [
  { id: 'giochi', label: 'Giochi', description: 'Azione, strategia, rompicapo e passatempi', icon: Gamepad2, hue: 340 },
  { id: 'internet', label: 'Internet', description: 'Browser, messaggistica, posta e condivisione', icon: Globe, hue: 212 },
  { id: 'audio-video', label: 'Audio e video', description: 'Musica, film, registrazione e montaggio', icon: Clapperboard, hue: 270 },
  { id: 'grafica', label: 'Grafica e foto', description: 'Disegno, fotoritocco, 3D e visualizzatori', icon: Image, hue: 24 },
  { id: 'ufficio', label: 'Ufficio', description: 'Documenti, fogli di calcolo, calendario e finanza', icon: Briefcase, hue: 150 },
  { id: 'sviluppo', label: 'Sviluppo', description: 'Editor, IDE, strumenti per programmare', icon: Code2, hue: 190 },
  { id: 'istruzione', label: 'Istruzione', description: 'Imparare lingue, materie e altro', icon: GraduationCap, hue: 45 },
  { id: 'scienza', label: 'Scienza', description: 'Matematica, astronomia, elettronica e dati', icon: FlaskConical, hue: 300 },
  { id: 'utilita', label: 'Utilità', description: 'Strumenti di tutti i giorni, file e terminale', icon: Wrench, hue: 100 },
  { id: 'sistema', label: 'Sistema', description: 'Monitoraggio, sicurezza e impostazioni', icon: Cpu, hue: 225 },
];

const byId = new Map(categories.map(category => [category.id, category]));
export const categoryInfo = (id: string): CategoryInfo | undefined => byId.get(id as CategoryId);

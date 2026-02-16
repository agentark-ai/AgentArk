import { Composition } from "remotion";
import { Main } from "./Main";

export const RemotionRoot: React.FC = () => {
  return (
    <Composition
      id="main"
      component={Main}
      width={1920}
      height={1080}
      fps={30}
      durationInFrames={300}
      defaultProps={{}}
    />
  );
};

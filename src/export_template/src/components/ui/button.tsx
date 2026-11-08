import { Button as Primitive } from '@base-ui/react/button'

/** Unstyled behavior primitive. Feature CSS supplies the visual treatment. */
export function Button({ type = 'button', ...props }: Primitive.Props) {
  return <Primitive type={type} {...props} />
}
